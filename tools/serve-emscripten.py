#!/usr/bin/env python3
"""Serve the Emscripten browser build with the isolation headers required by pthreads.

For a local browser on this machine, open http://localhost:8765/butaca-sim.html. If that port is
already in use, pass a free --port; do not serve these files with a generic static server, which
omits the isolation headers and prevents pthread workers from receiving SharedArrayBuffer. A LAN
IP over plain HTTP is not a secure context in mainstream browsers, so it cannot enable
SharedArrayBuffer; remote browsers need HTTPS with a trusted certificate. Pass --certfile/--keyfile
here or preserve these response headers through your TLS terminator.
"""
import argparse
import ssl
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


class IsolatedHandler(SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header("Cross-Origin-Opener-Policy", "same-origin")
        self.send_header("Cross-Origin-Embedder-Policy", "require-corp")
        self.send_header("Cache-Control", "no-store")
        super().end_headers()


def main():
    repo = Path(__file__).resolve().parent.parent
    default_dir = repo / "rust-modules/target/wasm32-unknown-emscripten/debug"
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="0.0.0.0", help="bind address (default: all interfaces)")
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--directory", type=Path, default=default_dir)
    parser.add_argument("--certfile", type=Path, help="TLS certificate; both TLS options are required")
    parser.add_argument("--keyfile", type=Path, help="TLS private key (keep it outside the repository)")
    parser.add_argument("--advertise-host", help="host name/IP to show in the URL when bound to all interfaces")
    args = parser.parse_args()
    if bool(args.certfile) != bool(args.keyfile):
        parser.error("--certfile and --keyfile must be provided together")
    directory = args.directory.resolve()
    if not (directory / "butaca-sim.html").is_file():
        parser.error(f"browser artifact not found: {directory / 'butaca-sim.html'} (run tools/build-emscripten.sh)")

    handler = partial(IsolatedHandler, directory=str(directory))
    server = ThreadingHTTPServer((args.host, args.port), handler)
    scheme = "http"
    if args.certfile:
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(args.certfile, args.keyfile)
        server.socket = context.wrap_socket(server.socket, server_side=True)
        scheme = "https"
    host_for_url = args.advertise_host or ("localhost" if args.host in ("0.0.0.0", "::") else args.host)
    print(f"Serving {directory} at {scheme}://{host_for_url}:{server.server_address[1]}/butaca-sim.html", flush=True)
    print("Pthreads need a secure origin: use localhost on this machine or HTTPS with a trusted certificate and these COOP/COEP headers for remote browsers.", flush=True)
    try:
        server.serve_forever(poll_interval=0.5)
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
