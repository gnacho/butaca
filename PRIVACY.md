# Butaca Privacy Policy

Applies to Butaca 0.7.0, the Jellyfin build published from this repository. Last updated
30 October 2026.

This page describes the package you install: a client for a Jellyfin server on a rooted LG webOS
television. The same source also builds the earlier Plex client, which is not what is published
here, and [the last section](#if-you-build-the-plex-flavour) covers how that one differs.

## Who is responsible

The maintainer of this repository is responsible for what Butaca stores on your television, and for
nothing else. Contact: `hnacho@proton.me`.

Butaca has no account with its author. There is nothing to sign up to, and no service of ours that
could receive anything about you.

## What it talks to

**The one server you configured, and nothing else.** Sign-in, library and metadata, artwork, video
and playback progress all go to that address over the transport it selects: HTTPS, or plain HTTP
when the address is an IP literal on your own network, which the app checks before it sends a
password.

There is no account service, no discovery service, no telemetry endpoint and no crash-reporting
endpoint compiled into this build, so `strings` on the binary settles it either way, and each
release audit reports what it found there.

Your server is operated by whoever runs it, which on a home network is normally you. That server's
own logs, its access rules and whether it is reachable from outside your network are its business,
not the app's. Butaca's author receives none of those requests.

## What it keeps on the television

- **`/media/developer/com.butaca-jellyfin.json`**, created mode `0600`: the servers you signed in to
  and which one is active. Each entry holds the address, the user, the access token the server
  issued, and a device identifier minted once so the server's device list does not grow on every
  boot. **It does not hold your password**: a password is used once at sign-in and the file is
  rewritten with the token, which the server can revoke on its own. Anyone who can read the
  television's filesystem can read that token, and the token grants access to your server.
- **The artwork cache.** Posters, backdrops and episode images fetched from your server are kept as
  bounded files beside the app, so the interface does not download them again on every launch.
- **The local logs**: `/tmp/butaca-events.log`, `/tmp/butaca-stderr.log` and
  `/tmp/butaca-crash.log`. The first two are truncated at each launch and the crash log is
  append-only so it survives a restart. Every line is scrubbed **before it is written**: tokens,
  header and query credentials, hostnames, bare addresses, search queries, and your server and user
  names are rewritten, while media titles, search terms and subtitle text are never written at all.
  Item identifiers do remain, because they are what makes a playback bug diagnosable. Someone with
  access to the same server could map one back to an item, so think before posting a log publicly.

Those files are the whole of it. A release build compiles out the development trigger surface, the
remote-control FIFO and the capture listener, and a crash writes no core file.

Butaca keeps no bookmark of its own for where you stopped watching: the playback position is held
by your server, which is also what carries it to another device.

## Signing out, deleting, uninstalling

**Disconnect**, the last row of the Servers page in Settings, removes the active server's entry:
its address, its user, its token and the device identifier it was given. If another server is left,
the app signs in to that one. If none is left, it erases the file and returns to the sign-in form.

**webOS gives an application no way to run code as it is removed.** The server list and the artwork
cache live beside the app rather than inside it, so removing the application can leave them behind.
Disconnect first if you want nothing of Butaca on the television, then remove the app. The logs live
in `/tmp`, which the television clears when it restarts.

## What is never collected

No crash reports, no analytics, no usage counters, no identifiers of any kind, and no access to what
you watch beyond the server the app points at. The reporting subsystem the earlier Plex client
carried was removed from this tree, so there is no code here that could send such a thing anywhere.

There is no switch to turn this off, because there is no reporting code left to switch off: the
subsystem is not in the tree, not merely disabled. If this page and the app ever disagree about what
leaves the television, this page is the one that counts, and the disagreement is a bug worth
reporting.

## If you build the Plex flavour

The Plex client is still in the tree and builds when the `jellyfin` feature is off. It is not the
package published here, and it differs in what it reaches and keeps:

- It signs in through **plex.tv**, which handles your Plex account and tells the app which servers
  that account can see. Plex processes what its own services receive under
  [Plex's privacy policy](https://www.plex.tv/about/privacy-legal/).
- It stores your Plex account token, a token for each server you use, and, for every profile you
  have switched to on this television, that profile's own server token and a one-way check computed
  from its PIN rather than the PIN itself, so that a switch still works with no internet.
- It also stores, for a server you were asked about, whether you allowed an unencrypted connection
  on your home network. Only the answer is kept, and it ends at sign-in or sign-out, at a profile
  switch unless the new profile reaches that server at the same address, or when the app returns
  from the background.
- It keeps the same local logs and the same artwork cache, scrubbed and bounded the same way, and it
  no longer carries crash or analytics reporting either.

## Contact and non-affiliation

Privacy questions: `hnacho@proton.me`. Security vulnerabilities may be reported privately through
GitHub Security Advisories for this repository.

Butaca is an independent, unofficial application. It is not produced by, endorsed by, or affiliated
with Plex, Inc., Jellyfin, or LG Electronics Inc.
