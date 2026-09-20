# butaca Privacy Policy

Applies to butaca (a fork of plx-native shipping the Jellyfin flavor). Last updated 20 September 2026.

## Who is responsible for butaca data

The butaca maintainers are responsible only for data butaca stores locally on this
television. Contact: `support@plxnative.com`.

## Your Jellyfin server

butaca is an independent client for Jellyfin. There is no central Jellyfin account service: to
sign you in, browse and play media, update watch progress and use server features, the app
communicates directly with the Jellyfin server you name. Those requests are handled by that
server and its operator. The butaca developers do not receive them.

## Data stored on this television

butaca stores the address of your Jellyfin server, your username and your password (in a file
only the app can read), a device identifier, your Home library choices, your recent searches and
your playback quality preference. It also keeps a small rotating local event log and a local
crash log for debugging. **Both logs stay on this television and are read only from it.**

**butaca sends nothing.** There are no crash reports, no product analytics and no diagnostics
leaving the television. Nothing the app records is transmitted to the butaca developers or to
any third party; there is no third-party processor, no analytics identifier and no report
queue.

It keeps no bookmark of its own for where you stopped watching: playback position is held by
your Jellyfin server. The Settings screen can sign out and remove butaca data from this
television.

## Lifetimes

Signing out removes the sign-in, the stored server address, username and password, and the
local settings. The event log rotates continuously, so its oldest lines are discarded; the
crash log is append-only until removed. Delete all local data removes everything butaca stored
on this television.

**webOS gives an application no way to run code as it is removed**, so the sign-in can survive
an uninstall (deliberately, so reinstalling does not sign you out). Use Delete all local data
before uninstalling if you want nothing of butaca left on this television.

## Uninstalling

Removing butaca removes the application. Anything kept outside the application's own directory
— your sign-in — can survive, as described above.

## Contact

Privacy questions: `support@plxnative.com`.
