# Product

<!-- impeccable:product-schema 1 -->

## Platform

Native LG webOS TV. One fixed 1920x1080 logical surface, controlled primarily with a television remote. This is not a web or Android application, and no responsive mobile viewport is in scope.

## Users

People using their own Jellyfin server from a television, normally from a sofa and with a remote. Their primary jobs are finding something to watch, understanding what is available, starting or resuming playback and controlling it without leaving the viewing experience.

## Product Purpose

Butaca is a production-quality native Jellyfin client for rooted LG webOS televisions. It makes a self-hosted media library feel like a first-class television product rather than a web page or a generic media grid.

## Positioning

The application combines native webOS playback and a purpose-built Rust interface with the user's own Jellyfin library. It is self-hosted, carries no telemetry and depends on no cloud account. Its value is the integration of media, remote-first navigation and television performance, not a catalogue of network or service-management features.

## Operating Context

The interface is viewed at a distance, often in a dark room, and driven by directional keys, OK, Back and playback keys. Focus must remain visible without relying on hover. Artwork comes from the user's actual server. A fixed 1920x1080 canvas, television text rasterization, hardware video-plane composition and bounded native performance are product constraints.

## Capabilities and Constraints

Existing navigation, library behavior, playback controls, settings and server workflows are preserved during the redesign. The fixed logical canvas and existing input model do not change. Native rendering uses the shared Rust theme, layout, widget and motion systems. No web stack, parallel visual primitives or cloud dependency is introduced.

## Brand Commitments

The existing Butaca name and product identity remain. The user has supplied the NetGrip Mobile visual guides as the binding reference for the redesign. Their graphite ground, amber signal, technical typography, disciplined surfaces and focus treatment guide the new visual world, translated to cinematic media rather than network-specific motifs.

## Evidence on Hand

- Current product captures under `docs/screenshots/` show the incumbent Home, Detail, Library, Settings and Player surfaces.
- The supplied NetGrip capture and source assets define the visual reference.
- The committed Jellyfin demo library supplies real, openly licensed content for simulator captures.
- No invented testimonials, commercial claims, customer evidence or cloud capabilities are needed.

## Product Principles

- Artwork and the act of watching lead. Interface chrome supports them and never competes with them.
- Every action remains understandable and reachable with the television remote.
- Legibility at sofa distance and predictable focus outrank decorative density.
- Self-hosted content, native playback and restrained performance cost are part of the product experience.
- One coherent visual language covers Home, Detail, Library, Settings and Player without weakening each surface's task.

## Accessibility & Inclusion

Focus is a first-class state. Ordinary content respects the existing couch-legibility floor, and controls never depend on hover, color alone or pointer precision. Long translations and server-provided metadata must remain readable without clipping or unexpected navigation changes.
