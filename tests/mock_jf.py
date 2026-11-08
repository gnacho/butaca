#!/usr/bin/env python3
"""A SYNTHETIC Jellyfin server — enough of the Jellyfin REST surface for the app to boot to Home
(flavor `jellyfin`), browse a library, open a detail page, a season, a person, search, and speak
the playback/user-data writes — with NOT ONE household byte anywhere in it.

Why it exists: campaign issue #166 (Jellyfin-only client). The committed replay fixtures are
Server-flavored because the controlled boot constructs a media-server world and no Jellyfin mock existed.
This server is the recording target the Jellyfin side was missing: every capture recorded against
it can be committed to a public repository without a scrub pass, because every content string is
drawn from one CLOSED ALPHABET — `s` + eight lowercase hex digits (`s3fa90c12`) — plus the
protocol's own constants (`Movie`, `h264`, `PG-13`, `Chapter 2`, …). Ids are 32-hex lowercase
strings derived from `--seed` with md5, the shape a real server's GUIDs take on the wire (no
dashes).

Deterministic: the world is generated from `--seed` and never from the clock. Two servers started
with one seed answer byte-identically; dates are fixed ISO strings derived from the seed, so no
volatile timestamp ever rides a payload a capture grades.

It is a contract server, not a Jellyfin emulator: it answers the routes
`rust-modules/src/jellyfin/client.rs` speaks with the FIELDS `jellyfin/dto.rs` deserialises, and
nothing else. An unknown path answers 404 and one stderr line naming it, so extending it is one
branch.

COVERED (route — used by):

  GET   /System/Info/Public                       — discovery/server name, Quick Connect gate
  POST  /Users/AuthenticateByName                 — password sign-in (any user/password accepted)
  POST  /QuickConnect/Initiate                    — quick_connect.rs
  GET   /QuickConnect/Connect?secret=             — the poll; authorized on the 2nd poll
  POST  /Users/AuthenticateWithQuickConnect       — the token trade
  GET   /Users/Me                                 — boot's stored-token probe (token required)
  GET   /Users/{uid}/Views                        — the library list (movies + tvshows views)
  GET   /Users/{uid}/Items/Resume                 — Home's Continue Watching
  GET   /Users/{uid}/Items/Latest                 — Home's Recently Added (BARE ARRAY, as current
                                                    servers answer; the client sniffs both shapes)
  GET   /Users/{uid}/Items                        — the listing workhorse: ParentId/Recursive/
                                                    IncludeItemTypes/SortBy/SortOrder/StartIndex/
                                                    Limit/IsPlayed/GenreIds/NameStartsWith/
                                                    searchTerm/PersonIds, Limit=0 counts
  GET   /Users/{uid}/Items/{id}                   — the detail fetch (Fields=… honoured: Genres,
                                                    People, Chapters, Taglines, ProductionLocations,
                                                    MediaSources, Trickplay)
  GET   /Genres?ParentId=                         — the filter menu
  GET   /Persons?searchTerm=                      — search's people half
  GET   /Shows/{id}/Seasons, /Shows/{id}/Episodes — the series detail page
  GET   /Shows/NextUp                             — Home's Next Up shelf and the detail OnDeck
  GET   /Items/{id}/Similar                       — the Related shelf
  GET   /MediaSegments/{id}                       — intro/credits; answers EMPTY (no markers)
  GET   /Items/{id}/Images/Primary|Backdrop/0|Logo — poster/art/logo bytes (api_key required): the
                                                    real derived artwork in --catalog mode, one
                                                    embedded 1x1 JPEG for the generated world
  POST  /Items/{id}/PlaybackInfo                  — the Media Decision Engine; always DIRECT play
  POST  /Sessions/Playing[/Progress|/Stopped]     — playback reporting; Stopped COMMITS the
                                                    resume point into UserData (visible in Resume)
  POST/DELETE /Users/{uid}/PlayedItems/{id}       — watched/unwatched writes (in-memory state)
  POST/DELETE /Users/{uid}/FavoriteItems/{id}     — favorite writes (in-memory state)
  POST  /Users/{uid}/Items/{id}/UserData          — the deck-removal MERGE (position only)
  GET   /Videos/{id}/stream                       - the static file (`?static=true`), Range honoured
  DELETE/Videos/ActiveEncodings                   — the transcode kill (204)
  GET/DELETE /_mock/requests                      — every routed request, token-redacted

NOT covered (the client folds their absence honestly): the HLS transcode playlist (playback
itself - the sim has no video path for it; it answers 404),
`/Videos/{id}/Trickplay/{w}/{i}.jpg` sheets (the manifest is served, the sheets 404),
`/Sessions/Capabilities/Full`, display-preferences, and admin routes. MediaSegments answers
empty rather than modelling an intro plugin, and `/Genres` answers the world's whole genre table
rather than one view's. AuthenticateByName never refuses a password (the 401 shape is graded by
the Rust unit suite); Quick Connect authorizes itself on the 2nd poll.

Usage:

    python3 tests/mock_jf.py --port 8920              # serve until Ctrl-C
    python3 tests/mock_jf.py --port 8920 --catalog    # serve the demo library instead
    python3 tests/mock_jf.py --port 8920 --selftest   # prove the shapes without an app

The app reaches it through the dev trigger (see `jellyfin/boot.rs`):
`$BUTACA_RUNTIME_DIR/butaca-jellyfin` containing
`{"url":"http://127.0.0.1:8920","user":"demo","password":"x"}` (`/tmp` on the television).

**The demo library (`--catalog`, [`CatalogLibrary`]).** The same wire and the same queries, built
from `tests/demo_library/catalog.json` and the artwork `tools/demo_library.py derive` renders into
`~/.cache/butaca-demo/derived/` (`$BUTACA_DEMO_CACHE` moves the cache). This is the world
`tools/screenshots.py` records against: real films, real posters and backdrops served as the bytes
on disk, and every id derived from the catalog's own slug, so a scene manifest can name a film
once and get one id in every run.
"""
import argparse
import base64
import hashlib
import json
import os
import pathlib
import random
import re
import sys
import threading
import time
import urllib.parse
from datetime import datetime, timedelta, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

# Where `tools/demo_library.py` keeps its manifests and its derived artwork cache.
DEMO_CATALOG = pathlib.Path(__file__).resolve().parent / "demo_library" / "catalog.json"

# One valid 1x1 JPEG — every image route serves these bytes. The poster store only scales and
# paints; a real server's pixels are content, and content is the closed alphabet's job to avoid.
JPEG_1X1 = base64.b64decode(
    "/9j/4AAQSkZJRgABAQEAYABgAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIs"
    "IxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/2wBDAQkJCQwLDBgNDRgyIRwhMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIy"
    "MjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjL/wAARCAABAAEDASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAA"
    "AAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAk"
    "M2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKT"
    "lJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/9oA"
    "DAMBAAIRAxEAPwD3+iiigD//2Q=="
)

OFFICIAL_RATINGS = ["G", "PG", "PG-13", "R", "TV-MA"]
VIDEO_CODECS = ["h264", "hevc"]
AUDIO_CODECS = ["aac", "eac3", "ac3"]

# The 100-ns tick arithmetic the wire speaks (client.rs' ms_to_ticks, inverted here).
TICKS_PER_MINUTE = 60 * 10_000_000


def sid(seed, *parts):
    """One stable 32-hex id, derived from the seed and the item's role — the wire shape of a
    Jellyfin GUID (the dto.rs fixture carries the same spelling), never the clock's."""
    return hashlib.md5(("jf:%d:%s" % (seed, ":".join(str(p) for p in parts))).encode()).hexdigest()


def sname(rng):
    """One name from the closed alphabet: `s` + 8 lowercase hex digits (the generated world's)."""
    return "s%08x" % rng.getrandbits(32)


def swords(rng, n):
    return " ".join(sname(rng) for _ in range(n))


def iso(dt):
    """The ISO-8601 shape Jellyfin puts on the wire (`1976-11-12T00:00:00.0000000Z`)."""
    return dt.strftime("%Y-%m-%dT%H:%M:%S.0000000Z")


def iso_date(text):
    """A `YYYY-MM-DD` catalog date as the same wire timestamp (midnight UTC)."""
    return "%sT00:00:00.0000000Z" % text


def demo_cache_dir():
    """The derived-artwork cache `tools/demo_library.py` writes (`$BUTACA_DEMO_CACHE` overrides)."""
    env = os.environ.get("BUTACA_DEMO_CACHE")
    return pathlib.Path(env) if env else pathlib.Path.home() / ".cache" / "butaca-demo"


def fallback_stream():
    """The clip an item with no film of its own still gets.

    The demo cache ships a short stand-in for the capture scenes; a machine that has not run
    `tools/demo_library.py derive` falls back to the source film. Returns None when neither
    exists, so the handler can answer 404 the way a server without the file would.

    A PATH, never the bytes: the source film is 282 MB, and reading it once per server was
    already more than the routes need.
    """
    cache = demo_cache_dir()
    for candidate in (cache / "derived" / "caminandes_1_2" / "stand-in.mp4",
                      cache / "src" / "sintel-film.mp4"):
        if candidate.is_file():
            return candidate
    return None


def date_at(rng, start_year, span_years):
    """One fixed date derived from the rng — no clock anywhere near a payload."""
    day = rng.randrange(0, span_years * 365)
    return datetime(start_year, 1, 1, tzinfo=timezone.utc) + timedelta(days=day)


# ------------------------------------------------------------------ the generated world -------

class Library:
    """Two views — movies and tvshows — plus box sets, people, genres and per-item watch state.
    Watch state follows fixed residue patterns so Resume/NextUp are non-empty but predictable:
    movies i%5==4 are played, i%7==3 in progress; series i%2==0 are STARTED (episode 1 watched,
    episode 2 in progress) so NextUp has both started and unstarted series."""

    def __init__(self, seed=1, movies=48, shows=6, seasons=2, episodes=4):
        self.seed = seed
        rng = random.Random(seed)
        self.server_id = sid(seed, "server")
        self.server_name = sname(rng)
        self.version = "10.11.11"  # >= 10.8: Quick Connect advertised
        self.user_id = sid(seed, "user")
        self.user_name = sname(rng)
        self.token = "jf" + hashlib.md5(("jf:tok:%d" % seed).encode()).hexdigest()[:10]
        self.views = [
            {"Id": sid(seed, "view", "movies"), "Name": sname(rng), "CollectionType": "movies"},
            {"Id": sid(seed, "view", "tv"), "Name": sname(rng), "CollectionType": "tvshows"},
        ]
        self.movies_view = self.views[0]["Id"]
        self.tv_view = self.views[1]["Id"]

        self.people = {}
        for i in range(1, 21):
            pid = sid(seed, "person", i)
            self.people[pid] = {
                "Id": pid, "Name": sname(rng), "Type": "Person",
                "PrimaryImageTag": sid(seed, "img", "person", i) if i % 3 else None,
            }
        self.people_by_index = list(self.people.keys())

        self.genres = {}
        for i in range(1, 9):
            gid = sid(seed, "genre", i)
            self.genres[gid] = {"Id": gid, "Name": "g%04x" % rng.getrandbits(16), "Type": "Genre"}

        self.items = {}  # id -> the wire dict (mutable: watch state lives here)
        self.images = {}  # id -> {"Primary"|"Backdrop"|"Logo": path}: real artwork, catalog mode only
        self._image_cache = {}  # path -> (content type, bytes)
        self.movies = []
        for i in range(1, movies + 1):
            item = self._movie(rng, i)
            self.items[item["Id"]] = item
            self.movies.append(item["Id"])
        self.series = []
        for i in range(1, shows + 1):
            show = self._series(rng, i, seasons, episodes)
            self.items[show["Id"]] = show
            self.series.append(show["Id"])
            for season in show["_seasons"]:
                self.items[season["Id"]] = season
                for episode in season["_episodes"]:
                    self.items[episode["Id"]] = episode
        self.boxsets = []
        for i in range(1, 4):
            box = self._boxset(rng, i)
            self.items[box["Id"]] = box
            self.boxsets.append(box["Id"])

    # ---- constructors -----------------------------------------------------------------

    def _genres_for(self, rng):
        picks = rng.sample(sorted(self.genres), rng.randrange(1, 4))
        return [self.genres[g]["Name"] for g in picks]

    def _people_for(self, rng, index):
        cast = []
        for role_n in range(rng.randrange(2, 5)):
            person = self.people[self.people_by_index[(index * 3 + role_n) % len(self.people)]]
            row = {"Name": person["Name"], "Id": person["Id"], "Type": "Actor",
                   "Role": sname(rng)}
            if person["PrimaryImageTag"]:
                row["PrimaryImageTag"] = person["PrimaryImageTag"]
            cast.append(row)
        director = self.people[self.people_by_index[(index * 7) % len(self.people)]]
        cast.append({"Name": director["Name"], "Id": director["Id"], "Type": "Director"})
        if index % 2 == 0:
            writer = self.people[self.people_by_index[(index * 5 + 1) % len(self.people)]]
            cast.append({"Name": writer["Name"], "Id": writer["Id"], "Type": "Writer"})
        return cast

    def _media_source(self, rng, item_id, runtime_minutes):
        video = VIDEO_CODECS[rng.randrange(2)]
        audio = AUDIO_CODECS[rng.randrange(3)]
        return {
            "Id": item_id,
            "Protocol": "File",
            "Container": "mkv",
            "Path": "/media/%s.mkv" % item_id,
            "Size": runtime_minutes * 60 * 4_000_000,
            "Bitrate": 4_000_000,
            "MediaStreams": [
                {"Type": "Video", "Codec": video, "Index": 0, "Width": 1920, "Height": 1080,
                 "AspectRatio": "16:9", "RealFrameRate": 24.0, "BitDepth": 8,
                 "VideoRangeType": "SDR", "BitRate": 3_500_000},
                {"Type": "Audio", "Codec": audio, "Index": 1, "Language": "eng",
                 "DisplayTitle": "Eng - %s - Stereo" % audio, "Channels": 6,
                 "ChannelLayout": "5.1", "BitRate": 384_000, "IsDefault": True},
                # A second audio track and two subtitle tracks on every other item, so the
                # detail hero's language discs and the picker have something to choose between
                # (the owner's pass). Deterministic on the item id, never random: a capture has
                # to show the same languages twice.
                *([{"Type": "Audio", "Codec": "aac", "Index": 2, "Language": "spa",
                    "DisplayTitle": "Spa - AAC - Stereo", "Channels": 2,
                    "ChannelLayout": "stereo", "BitRate": 192_000, "IsDefault": False}]
                  if int(item_id, 16) % 2 == 0 else []),
                {"Type": "Subtitle", "Codec": "subrip", "Index": 3, "Language": "eng",
                 "DisplayTitle": "Eng - SUBRIP", "IsDefault": True, "IsForced": False,
                 "IsExternal": False},
                *([{"Type": "Subtitle", "Codec": "subrip", "Index": 4, "Language": "spa",
                    "DisplayTitle": "Spa - SUBRIP", "IsDefault": False, "IsForced": False,
                    "IsExternal": False}]
                  if int(item_id, 16) % 2 == 0 else []),
            ],
        }

    def _movie(self, rng, index):
        item_id = sid(self.seed, "movie", index)
        runtime = rng.randrange(80, 141)
        premiere = date_at(rng, 1950, 75)
        created = date_at(rng, 2023, 3)
        played = index % 5 == 4
        in_progress = not played and index % 7 == 3
        last = iso(date_at(rng, 2024, 2)) if (played or in_progress) else None
        return {
            "Id": item_id,
            "Name": sname(rng),
            "Type": "Movie",
            "ProductionYear": premiere.year,
            "Overview": swords(rng, 6),
            "RunTimeTicks": runtime * TICKS_PER_MINUTE,
            "OfficialRating": OFFICIAL_RATINGS[rng.randrange(len(OFFICIAL_RATINGS))],
            "CommunityRating": round(rng.uniform(5.0, 9.5), 1),
            "PremiereDate": iso(premiere),
            "DateCreated": iso(created),
            "Genres": self._genres_for(rng),
            "People": self._people_for(rng, index),
            "ProductionLocations": [sname(rng)],
            "Taglines": [swords(rng, 2)],
            "ImageTags": {"Primary": sid(self.seed, "img", "movie", index)},
            "BackdropImageTags": ([sid(self.seed, "img", "moviebd", index)]
                                  if index % 10 else []),
            "Chapters": [
                {"Name": "Chapter %d" % (n + 1),
                 "StartPositionTicks": n * (runtime // 3) * TICKS_PER_MINUTE}
                for n in range(3)
            ],
            "UserData": {
                "Played": played,
                "PlayCount": 1 if (played or in_progress) else 0,
                "PlaybackPositionTicks": 0 if played else
                (runtime * TICKS_PER_MINUTE // 3 if in_progress else 0),
                "IsFavorite": index % 11 == 6,
                "LastPlayedDate": last,
            },
            "MediaSources": [self._media_source(rng, item_id, runtime)],
            "MediaType": "Video",
            "IsFolder": False,
        }

    def _series(self, rng, index, n_seasons, n_episodes):
        item_id = sid(self.seed, "series", index)
        created = date_at(rng, 2023, 3)
        started = index % 2 == 0
        seasons = []
        for s in range(1, n_seasons + 1):
            season = {
                "Id": sid(self.seed, "season", index, s),
                "Name": "Season %d" % s,
                "Type": "Season",
                "SeriesId": item_id,
                "SeriesName": None,  # filled once the show's name exists
                "IndexNumber": s,
                "ChildCount": n_episodes,
                "PremiereDate": iso(date_at(rng, 2015, 10)),
                "ImageTags": {"Primary": sid(self.seed, "img", "season", index, s)},
                "UserData": {"Played": False, "PlayCount": 0, "PlaybackPositionTicks": 0,
                             "UnplayedItemCount": n_episodes},
                "_episodes": [],
            }
            for e in range(1, n_episodes + 1):
                runtime = rng.randrange(22, 65)
                played = started and s == 1 and e == 1
                in_progress = started and s == 1 and e == 2
                episode_id = sid(self.seed, "episode", index, s, e)
                episode = {
                    "Id": episode_id,
                    "Name": sname(rng),
                    "Type": "Episode",
                    "SeriesId": item_id,
                    "ParentIndexNumber": s,
                    "IndexNumber": e,
                    "RunTimeTicks": runtime * TICKS_PER_MINUTE,
                    "PremiereDate": iso(date_at(rng, 2015, 10)),
                    "DateCreated": iso(created),
                    "Overview": swords(rng, 5),
                    "ImageTags": {"Primary": sid(self.seed, "img", "episode", index, s, e)},
                    "BackdropImageTags": [],
                    "UserData": {
                        "Played": played,
                        "PlayCount": 1 if (played or in_progress) else 0,
                        "PlaybackPositionTicks":
                            runtime * TICKS_PER_MINUTE * 2 // 5 if in_progress else 0,
                        "IsFavorite": False,
                        "LastPlayedDate": iso(date_at(rng, 2024, 2)) if started else None,
                    },
                    "MediaSources": [self._media_source(rng, episode_id, runtime)],
                    "MediaType": "Video",
                    "IsFolder": False,
                }
                season["_episodes"].append(episode)
            seasons.append(season)
        show = {
            "Id": item_id,
            "Name": sname(rng),
            "Type": "Series",
            "ProductionYear": 2015 + rng.randrange(10),
            "Overview": swords(rng, 6),
            "OfficialRating": OFFICIAL_RATINGS[rng.randrange(len(OFFICIAL_RATINGS))],
            "CommunityRating": round(rng.uniform(5.0, 9.5), 1),
            "PremiereDate": iso(date_at(rng, 2015, 10)),
            "EndDate": iso(date_at(rng, 2024, 2)) if index % 3 == 2 else None,
            "DateCreated": iso(created),
            "Genres": self._genres_for(rng),
            "People": self._people_for(rng, index + 100),
            "ChildCount": n_seasons,
            "ImageTags": {"Primary": sid(self.seed, "img", "series", index)},
            "BackdropImageTags": [sid(self.seed, "img", "seriesbd", index)],
            "UserData": {
                "Played": False, "PlayCount": 0, "PlaybackPositionTicks": 0,
                "IsFavorite": index % 11 == 6,
                "UnplayedItemCount": n_seasons * n_episodes - (1 if started else 0),
            },
        }
        for season in seasons:
            season["SeriesName"] = show["Name"]
            season["UserData"]["UnplayedItemCount"] = n_episodes - \
                (1 if started and season["IndexNumber"] == 1 else 0)
            for episode in season["_episodes"]:
                episode["SeriesName"] = show["Name"]
        show["_seasons"] = seasons
        return show

    def _boxset(self, rng, index):
        members = self.movies[: rng.randrange(2, 5)]
        return {
            "Id": sid(self.seed, "boxset", index),
            "Name": sname(rng),
            "Type": "BoxSet",
            "Overview": swords(rng, 4),
            "ChildCount": len(members),
            "PremiereDate": iso(date_at(rng, 2023, 3)),
            "DateCreated": iso(date_at(rng, 2023, 3)),
            "ImageTags": {"Primary": sid(self.seed, "img", "boxset", index)},
            "BackdropImageTags": [],
            "_members": members,
        }

    # ---- watch-state bookkeeping (the writes commit here, like the real server) ----------

    def mark_played(self, item_id):
        item = self.items.get(item_id)
        if not item or "UserData" not in item:
            return
        ud = item["UserData"]
        ud["Played"] = True
        ud["PlayCount"] = max(1, ud.get("PlayCount", 0) + 1)
        ud["PlaybackPositionTicks"] = 0

    def mark_unplayed(self, item_id):
        item = self.items.get(item_id)
        if not item or "UserData" not in item:
            return
        ud = item["UserData"]
        ud["Played"] = False
        ud["PlayCount"] = 0
        ud["PlaybackPositionTicks"] = 0

    def set_favorite(self, item_id, want):
        item = self.items.get(item_id)
        if item and "UserData" in item:
            item["UserData"]["IsFavorite"] = want

    def set_position(self, item_id, ticks):
        item = self.items.get(item_id)
        if item and "UserData" in item:
            item["UserData"]["PlaybackPositionTicks"] = max(0, ticks)

    def commit_stopped(self, item_id, ticks):
        """The Stopped report commits the resume point (client.rs documents the wire-verified
        behaviour); past a threshold the real server also plays the item — the mock only keeps
        the position, which is what Resume reads."""
        self.set_position(item_id, ticks)

    # ---- queries -------------------------------------------------------------------------

    def played(self, item):
        """A leaf is played off its own UserData; a series rolls up from its episodes (ALL
        played) — the rule client.rs' BrowseQuery doc states."""
        if item["Type"] == "Series":
            episodes = [self.items[e["Id"]] for s in item["_seasons"] for e in s["_episodes"]]
            return bool(episodes) and all(e["UserData"]["Played"] for e in episodes)
        return bool(item.get("UserData", {}).get("Played"))

    def series_episodes(self, series):
        return [self.items[e["Id"]] for s in series["_seasons"] for e in s["_episodes"]]

    def next_up_for(self, series):
        """The first unwatched episode of a STARTED series (some earlier episode watched), in
        season/episode order; `None` for a never-started or finished series."""
        episodes = self.series_episodes(series)
        for pos, episode in enumerate(episodes):
            if not episode["UserData"]["Played"]:
                return episode if pos > 0 and episodes[pos - 1]["UserData"]["Played"] else None
        return None

    def view_items(self, view_id, kinds):
        """The items of one view of one kind set, in stable id order (the listing sorts). A
        tvshows view owns its series AND their episodes; a movies view owns its movies and,
        for a BoxSet query, the box sets."""
        out = []
        if "Movie" in kinds:
            out += [self.items[i] for i in self.movies]
        if "Series" in kinds:
            out += [self.items[i] for i in self.series]
        if "Episode" in kinds:
            for i in self.series:
                out += self.series_episodes(self.items[i])
        if "BoxSet" in kinds:
            out += [self.items[i] for i in self.boxsets]
        return out

    def collection_children(self, boxset_id):
        """The members of one box set, in the server's own order for it — or `None` when the id is
        not a box set. A real server states a collection's order (custom where the owner pinned
        one) and `Items?ParentId=<boxset>` answers in it, so the pinned order survives the
        listing's `SortBy=SortName` rather than being re-sorted away."""
        box = self.items.get(boxset_id)
        if not box or box["Type"] != "BoxSet":
            return None
        return [self.items[i] for i in box["_members"]]

    def pinned_rank(self, boxset_id):
        """`{id: position}` when the library pins a custom order for that box set, else `None`."""
        return None

    def similar_for(self, item, limit):
        """The Related shelf's rows: the same kind, name-ordered — the generated world has no
        genre table worth ranking on, and a real server's own ranking is not reproduced here."""
        pool = [self.items[i] for i in self.movies + self.series
                if i != item["Id"] and self.items[i]["Type"] == item["Type"]]
        pool.sort(key=lambda it: it["Name"].lower())
        return pool[:limit]

    def image_bytes(self, item_id, kind):
        """`(content type, bytes)` of one image, or `None` when this item has no such image (the
        handler then answers its placeholder JPEG, which is what the generated world has for
        everything)."""
        path = self.images.get(item_id, {}).get(kind)
        if path is None:
            return None
        if path not in self._image_cache:
            ctype = "image/png" if path.suffix == ".png" else "image/jpeg"
            self._image_cache[path] = (ctype, path.read_bytes())
        return self._image_cache[path]


# ---------------------------------------------------------------- the demo catalog ----------

class CatalogLibrary(Library):
    """The demo library: the generated world's wire and queries, built from
    `tests/demo_library/catalog.json` and the artwork `tools/demo_library.py derive` renders
    instead of from a seed. This is the world `tools/screenshots.py` records against.

    Every id is derived from the catalog's own slug (`sid`, seed 0) — a movie, a show, a season,
    an episode and a box set each get one id that never moves, whatever the order of the catalog
    or the machine it runs on, so a scene manifest names a film (`@tears-of-steel`) once and the
    driver resolves the id it has today. `by_slug` is that map, in both directions of use:
    `"caminandes"` and `"caminandes/1/2"` are keys, their Jellyfin GUIDs the values.

    What the catalog states and the wire needs is lifted here: `sections` → the views,
    `collections` → box sets (with `collection_order` as a real server's custom order), shows →
    series/season/episode, `continue_watching`/`watched` → UserData, `added_order` → DateCreated
    and the derived artwork → the bytes `/Items/{id}/Images/…` answers. Nothing reads the clock:
    every date is the catalog's pinned `now`, so two runs answer byte-identically.
    """

    def __init__(self, catalog_path=DEMO_CATALOG, cache=None, hero=None):
        catalog_path = pathlib.Path(catalog_path)
        cat = json.loads(catalog_path.read_text())
        self.catalog = cat
        self.cache = pathlib.Path(cache) if cache else demo_cache_dir()
        self.derived = self.cache / "derived"
        if not self.derived.is_dir():
            raise ValueError("no derived demo artwork in %s: run `python3 tools/demo_library.py "
                             "derive` first" % self.derived)
        self.seed = 0
        self.now = int(cat["now"])
        self.server_id = cat["server"]["machineIdentifier"]
        self.server_name = cat["server"]["friendlyName"]
        self.version = "10.11.11"
        self.user_id = sid(self.seed, "user")
        self.user_name = "demo"
        self.token = "jf" + hashlib.md5(b"jf:tok:0").hexdigest()[:10]
        self.genres = {}
        self.people = {}
        self.people_by_index = []
        self.items = {}
        self.images = {}
        self.streams = {}
        self._image_cache = {}
        self.by_slug = {}
        self.sections = [dict(s, Id=sid(self.seed, "view", s["type"])) for s in cat["sections"]]
        self.views = [{"Id": s["Id"], "Name": s["title"],
                       "CollectionType": "movies" if s["type"] == "movie" else "tvshows"}
                      for s in self.sections]
        self.movies_view = self.views[0]["Id"]
        self.tv_view = self.views[1]["Id"] if len(self.views) > 1 else self.views[0]["Id"]
        self.movies, self.series, self.boxsets = [], [], []

        for rec in cat["movies"]:
            item = self._movie_item(rec)
            self.items[item["Id"]] = item
            self.movies.append(item["Id"])
        for rec in cat["shows"]:
            show = self._show_item(rec)
            self.items[show["Id"]] = show
            self.series.append(show["Id"])
            for season in show["_seasons"]:
                self.items[season["Id"]] = season
                for episode in season["_episodes"]:
                    self.items[episode["Id"]] = episode
        for name in cat["collections"]:
            box = self._boxset_item(name)
            self.items[box["Id"]] = box
            self.boxsets.append(box["Id"])

        self._pin_clock(hero or cat["hero"])
        self._roll_up()

    # ---- artwork -----------------------------------------------------------------------

    def _art(self, rec, slug):
        """The record's own derived artwork: `{"Primary"|"Backdrop"|"Logo": path}`. An entry the
        catalog names must have its bytes — a missing one is a cache that was never derived, which
        is worth failing on rather than serving a placeholder that hides it."""
        folder = self.derived / slug.replace("/", "_")
        out = {}
        for name, kind in (("poster", "Primary"), ("art", "Backdrop"), ("logo", "Logo")):
            path = folder / ("logo.png" if name == "logo" else "%s.jpg" % name)
            if name not in rec:
                continue
            if not path.is_file():
                raise ValueError("%s: derived %s is missing (%s); rerun "
                                 "`python3 tools/demo_library.py derive`" % (slug, name, path))
            out[kind] = path
        return out

    def _tag(self, *parts):
        return sid(self.seed, "imtag", *parts)

    # ---- the registries the queries filter on ------------------------------------------

    def _person(self, name):
        pid = sid(self.seed, "person", name)
        if pid not in self.people:
            self.people[pid] = {"Id": pid, "Name": name, "Type": "Person",
                                "PrimaryImageTag": None}
            self.people_by_index.append(pid)
        return self.people[pid]

    def _credit_rows(self, rec):
        """`People[]` in Jellyfin's own vocabulary: actors with their role, then the crew."""
        rows = []
        for entry in rec.get("cast", []):
            rows.append({"Name": entry["name"], "Id": self._person(entry["name"])["Id"],
                         "Type": "Actor", "Role": entry.get("role", "")})
        for name in rec.get("directors", []):
            rows.append({"Name": name, "Id": self._person(name)["Id"], "Type": "Director"})
        for name in rec.get("writers", []):
            rows.append({"Name": name, "Id": self._person(name)["Id"], "Type": "Writer"})
        return rows

    def _genre_names(self, rec):
        names = list(rec.get("genres", []))
        for name in names:
            gid = sid(self.seed, "genre", name)
            self.genres[gid] = {"Id": gid, "Name": name, "Type": "Genre"}
        return names

    def _stream_for(self, rec, slug):
        """The clip this record actually contains, when the cache carries one.

        `stand_in` records are the ones `tools/demo_library.py derive` renders a short clip for.
        A record with `media` plays the source asset it names: the catalog film is 14:48 and a
        scene that pauses at 7:26 needs those minutes to exist. Everything else has no film of
        its own and keeps the shared fallback, which is what the other scenes were written on.
        """
        if rec.get("stand_in"):
            path = self.derived / slug.replace("/", "_") / "stand-in.mp4"
            return path if path.is_file() else None
        asset = rec.get("media")
        if not asset:
            return None
        found = sorted((self.cache / "src").glob("%s.*" % asset))
        return found[0] if found else None

    # ---- the item builders -------------------------------------------------------------

    @staticmethod
    def _user_data():
        return {"Played": False, "PlayCount": 0, "PlaybackPositionTicks": 0,
                "IsFavorite": False, "LastPlayedDate": None}

    def _movie_item(self, rec):
        item_id = sid(self.seed, "movie", rec["id"])
        art = self._art(rec, rec["id"])
        self.images[item_id] = art
        self.by_slug[rec["id"]] = item_id
        minutes = rec["minutes"]
        stream = self._stream_for(rec, rec["id"])
        if stream:
            self.streams[item_id] = stream
        item = {
            "Id": item_id,
            "Name": rec["title"],
            "Type": "Movie",
            "ProductionYear": rec["year"],
            "Overview": rec.get("summary", ""),
            "RunTimeTicks": minutes * TICKS_PER_MINUTE,
            "DateCreated": self._pinned(3),
            "Genres": self._genre_names(rec),
            "People": self._credit_rows(rec),
            "ImageTags": {"Primary": self._tag("movie", rec["id"])},
            "BackdropImageTags": [self._tag("moviebd", rec["id"])] if "Backdrop" in art else [],
            "MediaSources": [self._media_source(random.Random(item_id), item_id, minutes)],
            "MediaType": "Video",
            "IsFolder": False,
            "UserData": self._user_data(),
            # The two numbers a film's hero states: an age rating and the audience score. Derived
            # from the id, so a capture shows the same ones twice and no catalog entry has to be
            # hand-edited to have a star.
            "OfficialRating": OFFICIAL_RATINGS[int(item_id, 16) % len(OFFICIAL_RATINGS)],
            "CommunityRating": round(5.0 + (int(item_id, 16) % 45) / 10.0, 1),
        }
        if "originallyAvailableAt" in rec:
            item["PremiereDate"] = iso_date(rec["originallyAvailableAt"])
        if rec.get("chapters"):
            item["Chapters"] = self.chapters(rec["chapters"], minutes * 60_000, rec["id"])
        return item

    def _show_item(self, rec):
        show_id = sid(self.seed, "series", rec["id"])
        art = self._art(rec, rec["id"])
        self.images[show_id] = art
        self.by_slug[rec["id"]] = show_id
        genres = self._genre_names(rec)
        show = {
            "Id": show_id,
            "Name": rec["title"],
            "Type": "Series",
            "ProductionYear": rec["year"],
            "Overview": rec.get("summary", ""),
            "PremiereDate": iso_date("%04d-01-01" % rec["year"]),
            "DateCreated": iso_date("%04d-01-01" % rec["year"]),
            "Genres": genres,
            "People": self._credit_rows(rec),
            "ChildCount": len(rec["seasons"]),
            "ImageTags": {"Primary": self._tag("series", rec["id"])},
            "BackdropImageTags": [self._tag("seriesbd", rec["id"])] if "Backdrop" in art else [],
            "UserData": self._user_data(),
            "_seasons": [],
        }
        for season in rec["seasons"]:
            index = season["index"]
            season_id = sid(self.seed, "season", rec["id"], index)
            self.images[season_id] = dict(art)
            self.by_slug["%s/%d" % (rec["id"], index)] = season_id
            episodes = []
            for entry in season["episodes"]:
                episode = self._episode_item(rec, season, entry, show_id, art)
                self.items[episode["Id"]] = episode
                episodes.append(episode)
            season_item = {
                "Id": season_id,
                "Name": "Season %d" % index,
                "Type": "Season",
                "SeriesId": show_id,
                "SeriesName": rec["title"],
                "IndexNumber": index,
                "ChildCount": len(episodes),
                "ImageTags": {"Primary": self._tag("season", rec["id"], index)},
                "UserData": dict(self._user_data(), UnplayedItemCount=len(episodes)),
                "_episodes": episodes,
            }
            if episodes and "PremiereDate" in episodes[0]:
                season_item["PremiereDate"] = episodes[0]["PremiereDate"]
            show["_seasons"].append(season_item)
        return show

    def _episode_item(self, show, season, entry, show_id, art):
        slug = "%s/%d/%d" % (show["id"], season["index"], entry["index"])
        item_id = sid(self.seed, "episode", show["id"], season["index"], entry["index"])
        still = self.derived / slug.replace("/", "_") / "thumb.jpg"
        if not still.is_file():
            raise ValueError("%s: derived thumb is missing (%s); rerun "
                             "`python3 tools/demo_library.py derive`" % (slug, still))
        images = {"Primary": still}
        if "Backdrop" in art:
            images["Backdrop"] = art["Backdrop"]
        self.images[item_id] = images
        self.by_slug[slug] = item_id
        minutes = entry["minutes"]
        stream = self._stream_for(entry, slug)
        if stream:
            self.streams[item_id] = stream
        return {
            "Id": item_id,
            "Name": entry["title"],
            "Type": "Episode",
            "SeriesId": show_id,
            "SeriesName": show["title"],
            "ParentIndexNumber": season["index"],
            "IndexNumber": entry["index"],
            "RunTimeTicks": minutes * TICKS_PER_MINUTE,
            "Overview": entry.get("summary", ""),
            "PremiereDate": iso_date(entry["date"]),
            "DateCreated": iso_date(entry["date"]),
            "Genres": list(show.get("genres", [])),
            "People": [],
            "ImageTags": {"Primary": self._tag("episode", show["id"], season["index"],
                                               entry["index"])},
            "BackdropImageTags": ([self._tag("seriesbd", show["id"])] if "Backdrop" in art
                                  else []),
            "MediaSources": [self._media_source(random.Random(item_id), item_id, minutes)],
            "MediaType": "Video",
            "IsFolder": False,
            "UserData": self._user_data(),
        }

    def _boxset_item(self, name):
        members = [m for m in self.catalog["movies"] if name in m.get("collections", [])]
        order = self.catalog.get("collection_order", {}).get(name)
        if order:
            rank = {slug: n for n, slug in enumerate(order)}
            members.sort(key=lambda m: rank.get(m["id"], len(rank)))
        else:
            members.sort(key=lambda m: self._added_index(m["id"]))
        box_id = sid(self.seed, "boxset", name)
        return {
            "Id": box_id,
            "Name": name,
            "Type": "BoxSet",
            "ChildCount": len(members),
            "DateCreated": self._pinned(4),
            "ImageTags": {},
            "BackdropImageTags": [],
            "UserData": self._user_data(),
            "_members": [self.by_slug[m["id"]] for m in members],
            "_pin": order,
        }

    # ---- the clock, the deck and the watch state --------------------------------------

    def _pinned(self, days):
        """One wire timestamp `days` before the catalog's pinned `now` — never the clock."""
        return iso(datetime.fromtimestamp(self.now - 86_400 * days, timezone.utc))

    def _added_index(self, slug):
        """Where the catalog's `added_order` puts one item (higher index = added EARLIER)."""
        order = self.catalog.get("added_order", [])
        return order.index(slug) if slug in order else len(order)

    def _pin_clock(self, hero):
        """DateCreated, the resume points and the watched flags from the catalog and its pinned
        `now`, never the wall clock. `hero` heads Continue Watching: the home hero pool opens with
        the deck, so that is its first slot (and `--hero` swaps one film for another without
        disturbing the rest of the order)."""
        if hero not in self.by_slug:
            raise ValueError("hero %r is not in the catalog" % hero)
        self.hero = hero
        for slug in self.catalog.get("added_order", []):
            item_id = self.by_slug.get(slug)
            if item_id is None:
                continue
            added = self.now - 86_400 * (2 + 3 * self._added_index(slug))
            item = self.items[item_id]
            item["DateCreated"] = self._pinned(2 + 3 * self._added_index(slug))
            # A show's own seasons and episodes sort under it, in their broadcast order, so the
            # library's Recently Added never interleaves one show's episodes into another's.
            for season in item.get("_seasons", []):
                for child in [season, *season["_episodes"]]:
                    step = (season.get("IndexNumber", 0) * 100
                            + child.get("IndexNumber", 0)) * 60
                    child["DateCreated"] = iso(
                        datetime.fromtimestamp(added + step, timezone.utc))
        deck = [dict(c) for c in self.catalog.get("continue_watching", [])]
        if all(c["item"] != hero for c in deck):
            deck.insert(0, {"item": hero, "progress": 0.42})
        deck.sort(key=lambda c: c["item"] != hero)  # stable: the hero first, the rest in order
        for n, entry in enumerate(deck):
            item = self.items[self.by_slug[entry["item"]]]
            item["UserData"]["PlaybackPositionTicks"] = \
                int(item["RunTimeTicks"] * entry["progress"])
            item["UserData"]["LastPlayedDate"] = iso(
                datetime.fromtimestamp(self.now - 600 - 3_600 * n, timezone.utc))
        for n, slug in enumerate(self.catalog.get("watched", [])):
            item = self.items[self.by_slug[slug]]
            item["UserData"]["Played"] = True
            item["UserData"]["PlayCount"] = 1
            item["UserData"]["PlaybackPositionTicks"] = 0
            item["UserData"]["LastPlayedDate"] = iso(
                datetime.fromtimestamp(self.now - 86_400 * (10 + n), timezone.utc))

    def _roll_up(self):
        """The containers' `UnplayedItemCount` and `ChildCount` from the leaves under them, as the
        real server aggregates them — the series row's watched state and the detail page's
        "N episodes left" both read this and nothing else."""
        for series_id in self.series:
            show = self.items[series_id]
            episodes = self.series_episodes(show)
            left = sum(1 for e in episodes if not e["UserData"]["Played"])
            show["ChildCount"] = len(show["_seasons"])
            show["UserData"]["UnplayedItemCount"] = left
            show["UserData"]["Played"] = bool(episodes) and left == 0
            show["UserData"]["PlayCount"] = int(show["UserData"]["Played"])
            for season in show["_seasons"]:
                left = sum(1 for e in season["_episodes"] if not e["UserData"]["Played"])
                season["UserData"]["UnplayedItemCount"] = left
                season["UserData"]["Played"] = left == 0

    # ---- what the catalog states about order and relatedness --------------------------

    def pinned_rank(self, boxset_id):
        box = self.items.get(boxset_id)
        order = (box or {}).get("_pin") or None
        if not order:
            return None
        return {self.by_slug[slug]: n for n, slug in enumerate(order)}

    def similar_for(self, item, limit):
        """Related rows by shared genre, then name — what a real server's similar list is
        approximately about, and enough for the shelf to be worth looking at."""
        mine = set(item.get("Genres", []))
        pool = [self.items[i] for i in self.movies + self.series
                if i != item["Id"] and self.items[i]["Type"] == item["Type"]]
        pool.sort(key=lambda it: (-len(mine.intersection(it.get("Genres", []))),
                                 it["Name"].lower()))
        return pool[:limit]

    @staticmethod
    def chapters(marks, duration, slug):
        """`[{"start": "m:ss", "title": …}, …]` from the catalog → the server's `Chapters[]`, as
        the tick offsets the detail page's strip reads. The first must start at 0:00 and the
        starts must rise inside the film, as they do on a real file."""
        def ms(stamp):
            minutes, seconds = stamp.split(":")
            return (int(minutes) * 60 + int(seconds)) * 1000

        starts = [ms(c["start"]) for c in marks]
        if marks and (starts[0] != 0 or starts != sorted(set(starts)) or starts[-1] >= duration):
            raise ValueError("%s: chapter starts must begin at 0:00, rise, and end inside the "
                             "film" % slug)
        return [{"Name": c["title"], "StartPositionTicks": start * 10_000}
                for c, start in zip(marks, starts)]


# ------------------------------------------------------------------ the server ----------------

SORT_KEYS = {
    "SortName": lambda it: it["Name"].lower(),
    "PremiereDate": lambda it: it.get("PremiereDate") or "",
    "DateCreated": lambda it: it.get("DateCreated") or "",
    "CommunityRating": lambda it: it.get("CommunityRating") or 0.0,
}

LISTING_FIELDS = "Overview,MediaSources,OfficialRating,CommunityRating"
DETAIL_FIELDS = ("MediaSources,Overview,Genres,People,Chapters,Taglines,"
                 "ProductionLocations,OfficialRating,CommunityRating")


def wire_row(item):
    """The public JSON of one item — the leading-underscore bookkeeping keys never travel."""
    return {k: v for k, v in item.items() if not k.startswith("_")}


class Server(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, addr, lib):
        super().__init__(addr, Handler)
        self.lib = lib
        self.lock = threading.Lock()
        self.requests = []       # {"method","path","query","status"} — token values redacted
        self.unknown = []        # paths that fell through handle()
        self.qc = {}             # quick-connect secret -> {"polls", "code"}
        self.write_log = []      # ["POST /Users/{uid}/PlayedItems/{id}", …] — the mock write log


def redacted_query(query):
    out = {}
    for key, values in query.items():
        if "token" in key.lower() or "api_key" in key.lower() or key == "secret":
            out[key] = "<redacted>"
        else:
            out[key] = values
    return out


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "MockJF/0"

    def log_message(self, fmt, *args):
        pass  # _do logs a redacted route; BaseHTTPRequestHandler would log tokens.

    # ---- plumbing ----------------------------------------------------------------------

    def _send_json(self, status, payload):
        """One JSON answer. Returns the status so handle() can report it to the request log.
        A body left unread RSTs the socket under the client's response read on some kernels
        (the flake class `rust-modules/src/jellyfin/mock_server.rs` describes) — `_do` drains
        every body."""
        data = b"" if payload is None else \
            json.dumps(payload, separators=(",", ":")).encode()
        self.send_response(status)
        if payload is not None:
            self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        if data:
            self.wfile.write(data)
        return status

    def _send_bytes(self, status, data, ctype):
        self.send_response(status)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)
        return status

    def _token(self, query):
        """The request's token, from the one carrier the client uses per route class: the
        `Authorization: MediaBrowser Token="…"` header on control routes, `api_key=` on built
        media/image paths. Never logged."""
        match = re.search(r'Token="([^"]+)"', self.headers.get("Authorization") or "")
        if match:
            return match.group(1)
        if "X-Emby-Token" in self.headers:
            return self.headers.get("X-Emby-Token")
        values = query.get("api_key", [])
        return values[0] if values else None

    def _record(self, method, path, query, status):
        with self.server.lock:
            self.server.requests.append({
                "method": method,
                "path": path,
                "query": redacted_query(query),
                "status": status,
            })

    # ---- dispatch ----------------------------------------------------------------------

    def _do(self, method):
        u = urllib.parse.urlsplit(self.path)
        query = urllib.parse.parse_qs(u.query, keep_blank_values=True)
        n = int(self.headers.get("Content-Length") or 0)
        self.request_body = self.rfile.read(n) if n else b""
        status = self.route(method, u.path, query)
        self._record(method, u.path, query, status)

    def do_GET(self):
        self._do("GET")

    def do_HEAD(self):
        self._do("HEAD")

    def do_POST(self):
        self._do("POST")

    def do_PUT(self):
        self._do("PUT")

    def do_DELETE(self):
        self._do("DELETE")

    def route(self, method, path, query):
        lib = self.server.lib
        segs = [s for s in path.split("/") if s]

        # ---- public / unauthenticated -------------------------------------------------
        if path == "/System/Info/Public" and method == "GET":
            return self._send_json(200, {"ServerName": lib.server_name, "Id": lib.server_id,
                                         "Version": lib.version})
        if path == "/Users/AuthenticateByName" and method == "POST":
            try:
                creds = json.loads(self.request_body) if self.request_body else {}
            except ValueError:
                creds = {}
            # Any name/password is accepted (the module doc's simplification); the reply shape
            # is what the client parses, and 401 refusal is graded by the Rust unit suite.
            print("mock_jf: AuthenticateByName user=%s" % creds.get("Username", "?"),
                  file=sys.stderr, flush=True)
            return self._send_json(200, {
                "AccessToken": lib.token,
                "User": {"Id": lib.user_id, "Name": lib.user_name},
                "ServerId": lib.server_id,
                "SessionInfo": None,
            })
        if path == "/QuickConnect/Initiate" and method == "POST":
            secret = sid(lib.seed, "qc", len(self.server.qc) + 1)
            code = "%06d" % (424000 + len(self.server.qc) + 1)
            with self.server.lock:
                self.server.qc[secret] = {"polls": 0, "code": code}
            return self._send_json(200, {"Secret": secret, "Code": code,
                                         "Authenticated": False})
        if path == "/QuickConnect/Connect" and method == "GET":
            secret = (query.get("secret") or [""])[0]
            with self.server.lock:
                state = self.server.qc.get(secret)
                if state is None:
                    return self._send_json(404, {})
                state["polls"] += 1
                authorized = state["polls"] >= 2  # authorized on the 2nd poll
                code = state["code"]
            return self._send_json(200, {"Secret": secret, "Code": code,
                                         "Authenticated": authorized})
        if path == "/Users/AuthenticateWithQuickConnect" and method == "POST":
            try:
                payload = json.loads(self.request_body) if self.request_body else {}
            except ValueError:
                payload = {}
            with self.server.lock:
                authorized = self.server.qc.get(payload.get("Secret"), {}).get("polls", 0) >= 2
            if not authorized:
                return self._send_json(401, {})
            return self._send_json(200, {
                "AccessToken": lib.token,
                "User": {"Id": lib.user_id, "Name": lib.user_name},
                "ServerId": lib.server_id,
                "SessionInfo": None,
            })

        # ---- token-required routes ------------------------------------------------------
        # Accepted, never checked — the generated world's own rule for its credentials
        # (its `butaca-token` line): the carrier SHAPE is what the client's correctness lives on,
        # and the 401 below is the absence answer. The controlled replay world's fixture client
        # (#166 flip) carries a token the mock cannot have minted, and a recording must reach the
        # same answers a real sign-in would.
        if not self._token(query):
            return self._send_json(401, {})

        if path == "/Users/Me" and method == "GET":
            return self._send_json(200, {"Id": lib.user_id, "Name": lib.user_name})

        if path == "/_mock/requests" and method == "GET":
            with self.server.lock:
                return self._send_json(200, list(self.server.requests))
        if path == "/_mock/requests" and method == "DELETE":
            with self.server.lock:
                self.server.requests.clear()
            return self._send_json(200, {})

        # /Users/{uid}/Views
        if len(segs) == 3 and segs[0] == "Users" and segs[2] == "Views" and method == "GET":
            return self._send_json(200, {"Items": [dict(v) for v in lib.views]})

        # /Users/{uid}/Items…
        if len(segs) >= 3 and segs[0] == "Users" and segs[2] == "Items":
            tail = segs[3] if len(segs) > 3 else None
            if tail is None and method == "GET":
                return self.items_page(lib, query)
            if tail == "Resume" and method == "GET":
                return self.items_resume(lib, query)
            if tail == "Latest" and method == "GET":
                return self.items_latest(lib, query)
            if tail and len(segs) == 4 and method == "GET":
                return self.item_detail(lib, tail, query)
            if tail and len(segs) == 5 and segs[4] == "UserData" and method == "POST":
                try:
                    payload = json.loads(self.request_body) if self.request_body else {}
                except ValueError:
                    payload = {}
                lib.set_position(tail, int(payload.get("PlaybackPositionTicks") or 0))
                with self.server.lock:
                    self.server.write_log.append("%s %s" % (method, path))
                return self._send_json(200, {})

        # /Users/{uid}/PlayedItems/{id} and /Users/{uid}/FavoriteItems/{id}
        if len(segs) == 4 and segs[0] == "Users" and segs[2] in ("PlayedItems", "FavoriteItems"):
            if segs[2] == "PlayedItems":
                if method == "POST":
                    lib.mark_played(segs[3])
                elif method == "DELETE":
                    lib.mark_unplayed(segs[3])
                else:
                    return self._send_json(405, {})
            elif method in ("POST", "DELETE"):
                lib.set_favorite(segs[3], method == "POST")
            else:
                return self._send_json(405, {})
            with self.server.lock:
                self.server.write_log.append("%s %s" % (method, path))
            return self._send_json(200, {})

        if path == "/Genres" and method == "GET":
            genres = sorted(lib.genres.values(), key=lambda g: g["Name"].lower())
            return self._send_json(200, {"Items": genres, "TotalRecordCount": len(genres)})

        if path == "/Persons" and method == "GET":
            terms = (query.get("searchTerm") or [""])[0].lower()
            rows = [{"Id": p["Id"], "Name": p["Name"], "Type": "Person",
                     "ImageTags": ({"Primary": p["PrimaryImageTag"]}
                                   if p["PrimaryImageTag"] else {})}
                    for p in lib.people.values() if terms and terms in p["Name"].lower()]
            return self._send_json(200, {"Items": rows, "TotalRecordCount": len(rows)})

        if path == "/Shows/NextUp" and method == "GET":
            return self.next_up(lib, query)

        if len(segs) == 3 and segs[0] == "Shows" and segs[2] == "Seasons" and method == "GET":
            series = lib.items.get(segs[1])
            if not series or series["Type"] != "Series":
                return self._send_json(404, {})
            rows = [wire_row(s) for s in series["_seasons"]]
            return self._send_json(200, {"Items": rows, "TotalRecordCount": len(rows)})
        if len(segs) == 3 and segs[0] == "Shows" and segs[2] == "Episodes" and method == "GET":
            series = lib.items.get(segs[1])
            season_id = (query.get("SeasonId") or [""])[0]
            if not series or series["Type"] != "Series":
                return self._send_json(404, {})
            episodes = []
            for season in series["_seasons"]:
                if season_id and season["Id"] != season_id:
                    continue
                episodes += [wire_row(e) for e in season["_episodes"]]
            return self._send_json(200, {"Items": episodes, "TotalRecordCount": len(episodes)})

        if len(segs) == 3 and segs[0] == "Items" and segs[2] == "Similar" and method == "GET":
            item = lib.items.get(segs[1])
            if not item:
                return self._send_json(404, {})
            limit = int((query.get("Limit") or ["12"])[0] or 12)
            rows = [wire_row(it) for it in lib.similar_for(item, limit)]
            return self._send_json(200, {"Items": rows, "TotalRecordCount": len(rows)})
        if len(segs) == 3 and segs[0] == "Items" and segs[2] == "PlaybackInfo" \
                and method == "POST":
            item = lib.items.get(segs[1])
            if not item:
                return self._send_json(404, {})
            source_id = item["MediaSources"][0]["Id"] if item.get("MediaSources") else item["Id"]
            return self._send_json(200, {
                "MediaSources": [{"Id": source_id, "SupportsDirectStream": True,
                                  "SupportsTranscoding": True}],
                "PlaySessionId": sid(lib.seed, "ps", segs[1]),
            })
        if len(segs) >= 4 and segs[0] == "Items" and segs[2] == "Images" and method == "GET":
            return self.item_image(lib, segs[1], segs[3], query)
        if len(segs) == 3 and segs[0] == "Items" and segs[2] in ("LocalTrailers",
                                                                  "SpecialFeatures") \
                and method == "GET":
            # Honest emptiness: the demo library carries no extras, and the client folds an empty
            # list to "none" exactly as it folds a 404 (#387 reads both routes on every detail).
            return self._send_json(200, {"Items": [], "TotalRecordCount": 0})
        if path == "/Plugins/MdbListRatings/CachedByItemId" and method == "GET":
            # The MDBList plugin is not installed on a demo server: a 404 is its answer, and the
            # client already treats it as "no scores from that source".
            return self._send_json(404, {})
        if len(segs) == 2 and segs[0] == "MediaSegments" and method == "GET":
            # Honest emptiness: the mock models no intro plugin; the client folds this to "no
            # markers" exactly as it folds a 404.
            return self._send_json(200, {"Items": []})

        if path in ("/Sessions/Playing", "/Sessions/Playing/Progress",
                    "/Sessions/Playing/Stopped") and method == "POST":
            return self.session_report(lib, path)

        if path == "/Videos/ActiveEncodings" and method == "DELETE":
            return self._send_json(204, None)

        if len(segs) == 3 and segs[0] == "Videos" and segs[2] in ("stream", "stream.mp4"):
            return self.video_stream(lib, segs[1])

        with self.server.lock:
            self.server.unknown.append("%s %s" % (method, path))
        print("mock_jf: UNKNOWN %s %s" % (method, path), file=sys.stderr, flush=True)
        return self._send_json(404, {})

    # ---- route bodies -------------------------------------------------------------------

    def items_resume(self, lib, query):
        """Home's Continue Watching: in-progress movies and episodes, server-sorted by
        last-played — the deck's own order (client.rs' resume() doc)."""
        limit = int((query.get("Limit") or ["20"])[0] or 20)
        pool = [lib.items[i] for i in lib.movies]
        for i in lib.series:
            pool += lib.series_episodes(lib.items[i])
        rows = [it for it in pool
                if 0 < it["UserData"]["PlaybackPositionTicks"] and not it["UserData"]["Played"]]
        rows.sort(key=lambda it: it["UserData"].get("LastPlayedDate") or "", reverse=True)
        rows = [wire_row(it) for it in rows[:limit]]
        return self._send_json(200, {"Items": rows, "TotalRecordCount": len(rows)})

    def items_latest(self, lib, query):
        """The BARE ARRAY shape current servers answer (client.rs sniffs both). Scoped to the
        view: the movies view answers movies, the tvshows view answers SERIES (Latest collapses
        episodes to their series — the shelf shape the client relies on)."""
        parent = (query.get("ParentId") or [""])[0]
        limit = int((query.get("Limit") or ["20"])[0] or 20)
        pool = [lib.items[i] for i in (lib.series if parent == lib.tv_view else lib.movies)]
        pool.sort(key=lambda it: it.get("DateCreated") or "", reverse=True)
        return self._send_json(200, [wire_row(it) for it in pool[:limit]])

    def items_page(self, lib, query):
        """The listing workhorse — every Library-screen, search and filmography query lands
        here. Limit=0 answers the count with no items (the A–Z rail's probe)."""
        parent = (query.get("ParentId") or [""])[0]
        include = (query.get("IncludeItemTypes") or [""])[0]
        kinds = [k for k in include.split(",") if k] or ["Movie", "Series"]
        start = int((query.get("StartIndex") or ["0"])[0] or 0)
        limit = int((query.get("Limit") or ["50"])[0] or 50)
        sort_by = (query.get("SortBy") or ["SortName"])[0]
        descending = (query.get("SortOrder") or ["Ascending"])[0] == "Descending"
        unwatched_only = any(v.lower() == "false" for v in query.get("IsPlayed", []))
        genre_ids = [g for v in query.get("GenreIds", []) for g in v.split(",") if g]
        starts = (query.get("NameStartsWith") or [None])[0]
        terms = (query.get("searchTerm") or [None])[0]
        person_ids = [p for v in query.get("PersonIds", []) for p in v.split(",") if p]

        pool = [lib.items[i] for i in lib.boxsets] if kinds == ["BoxSet"] \
            else lib.view_items(parent, kinds)
        children = lib.collection_children(parent) if parent else None
        if children is not None:
            # `Items?ParentId=<boxset>` — the collection screen's own member listing.
            pool = children

        if person_ids:
            pool = [it for it in pool
                    if any(p["Id"] in person_ids for p in it.get("People", []))]
        if terms is not None:
            pool = [it for it in pool if terms.lower() in it["Name"].lower()]
        if unwatched_only:
            pool = [it for it in pool if not lib.played(it)]
        if genre_ids:
            by_name = {lib.genres[g]["Name"] for g in genre_ids if g in lib.genres}
            pool = [it for it in pool if by_name.intersection(it.get("Genres", []))]
        if starts:
            pool = [it for it in pool if it["Name"].lower().startswith(starts.lower())]

        key = SORT_KEYS.get(sort_by, SORT_KEYS["SortName"])
        pinned = lib.pinned_rank(parent) if parent else None
        if pinned:
            # A pinned collection order IS the server's order for that listing; the client still
            # sends SortBy=SortName and still gets the collection as the owner arranged it.
            pool = sorted(pool, key=lambda it: pinned.get(it["Id"], len(pinned)))
        else:
            pool = sorted(pool, key=lambda it: (key(it), it["Name"].lower()), reverse=descending)

        total = len(pool)
        window = pool[start:start + limit] if limit > 0 else []
        return self._send_json(200, {"Items": [wire_row(it) for it in window],
                                     "TotalRecordCount": total})

    def item_detail(self, lib, item_id, query):
        item = lib.items.get(item_id)
        if not item:
            return self._send_json(404, {})
        row = wire_row(item)
        if "Trickplay" in (query.get("Fields") or [""])[0] and item["Type"] == "Movie":
            # One tile-width entry, exactly the shape trickplay_info() parses.
            row["Trickplay"] = {"320": {"0": {
                "Width": 320, "Height": 180, "TileWidth": 10, "TileHeight": 10,
                "ThumbnailCount": 100, "Interval": 10000, "Bandwidth": 123456}}}
        return self._send_json(200, row)

    def video_stream(self, lib, item_id):
        """The static file the client plays (`?static=true`). Range is honoured because the
        player seeks: a server that ignored the header answers the whole file and the player
        looks like it stalled.

        The path comes from the ITEM: handing every item the same clip made a scene that
        pauses at 7:26 of a 14:48 film reach the end of a three-minute stand-in instead.
        """
        # The synthetic world carries no clips of its own; only the demo catalog does.
        path = getattr(lib, "streams", {}).get(item_id) or fallback_stream()
        if path is None:
            return self._send_json(404, {})
        total = path.stat().st_size
        start, end = 0, total - 1
        status = 200
        match = re.match(r"bytes=(\d*)-(\d*)$", (self.headers.get("Range") or "").strip())
        if match:
            first, last = match.group(1), match.group(2)
            if first:
                start = int(first)
                end = min(int(last), total - 1) if last else total - 1
            elif last:
                start = max(total - int(last), 0)
            if start > end or start >= total:
                self.send_response(416)
                self.send_header("Content-Range", "bytes */%d" % total)
                self.send_header("Content-Length", "0")
                self.end_headers()
                return 416
            status = 206
        length = end - start + 1
        self.send_response(status)
        self.send_header("Content-Type", "video/mp4")
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Length", str(length))
        if status == 206:
            self.send_header("Content-Range", "bytes %d-%d/%d" % (start, end, total))
        self.end_headers()
        remaining = length
        with path.open("rb") as fh:
            fh.seek(start)
            while remaining > 0:
                block = fh.read(min(64 * 1024, remaining))
                if not block:
                    break
                self.wfile.write(block)
                remaining -= len(block)
        return status

    def item_image(self, lib, item_id, kind_path, query):
        # The built path carries api_key=… as its only credential (client.rs' rule). Accepted,
        # never checked — the same rule as every token-required route above.
        if not self._token(query):
            return self._send_json(401, {})
        if not kind_path.startswith(("Primary", "Backdrop", "Logo", "Art", "Thumb")):
            return self._send_json(404, {})
        if item_id not in lib.items and item_id not in lib.people:
            return self._send_json(404, {})
        kind = kind_path.split("/")[0]
        found = lib.image_bytes(item_id, kind)
        if found is None:
            return self._send_bytes(200, JPEG_1X1, "image/jpeg")
        return self._send_bytes(200, found[1], found[0])

    def next_up(self, lib, query):
        limit = int((query.get("Limit") or ["1"])[0] or 1)
        series_id = (query.get("SeriesId") or [None])[0]
        shows = [lib.items[series_id]] if series_id in lib.items \
            else [lib.items[i] for i in lib.series]
        rows = []
        for show in shows:
            episode = lib.next_up_for(show)
            if episode:
                rows.append(wire_row(episode))
        return self._send_json(200, {"Items": rows[:limit], "TotalRecordCount": len(rows)})

    def session_report(self, lib, path):
        try:
            payload = json.loads(self.request_body) if self.request_body else {}
        except ValueError:
            payload = {}
        item_id = payload.get("ItemId")
        ticks = int(payload.get("PositionTicks") or 0)
        if path.endswith("/Stopped") and item_id:
            lib.commit_stopped(item_id, ticks)
        return self._send_json(204, None)


# ------------------------------------------------------------------ entry point ---------------

def serve(port, seed=1, host="127.0.0.1", movies=48, shows=6, catalog=None,
          catalog_cache=None, hero=None):
    """Start the mock and return `(srv, lib)`; the caller runs `srv.serve_forever()` (a thread in
    the tools and the tests, the foreground in `main`). `catalog` serves the demo library
    (`tests/demo_library/catalog.json` by default) instead of a generated seed."""
    if catalog is not None:
        lib = CatalogLibrary(DEMO_CATALOG if catalog is True else catalog,
                             cache=catalog_cache, hero=hero)
    else:
        lib = Library(seed=seed, movies=movies, shows=shows)
    return Server((host, port), lib), lib


def run(srv):
    """Serve `srv` on a daemon thread — the one line a tool or a test needs to make it answer."""
    thread = threading.Thread(target=srv.serve_forever, args=(0.2,), daemon=True)
    thread.start()
    return thread


def selftest(catalog=None):
    """Prove the shapes without an app: every covered route answers, and the fields the Rust
    DTOs deserialise are present with the right JSON types — the contract mock_jf exists for.
    With the derived artwork cache present it proves the demo catalog's world too (the one
    `tools/screenshots.py` records against); without it, that half is skipped by name."""
    import urllib.error
    import urllib.request

    srv, lib = serve(0, seed=7)
    run(srv)
    port = srv.server_address[1]
    base = "http://127.0.0.1:%d" % port
    failures = []

    def request(method, path, body=None, token=None, want_status=200):
        req = urllib.request.Request(base + path, data=body, method=method)
        if token:
            req.add_header(
                "Authorization",
                'MediaBrowser Token="%s", Client="T", Device="D", DeviceId="dev", Version="1"'
                % token)
        try:
            with urllib.request.urlopen(req, timeout=5) as reply:
                status, data = reply.status, reply.read()
        except urllib.error.HTTPError as error:
            status, data = error.code, error.read()
        if status != want_status:
            failures.append("%s %s -> %s (wanted %s)" % (method, path, status, want_status))
            return None
        return json.loads(data) if data and data[:1] in (b"{", b"[") else data

    info = request("GET", "/System/Info/Public")
    assert info["ServerName"] and re.fullmatch(r"10\.\d+\.\d+", info["Version"]), info

    # A control route without a token is refused; /Users/Me is the boot's validity probe.
    request("GET", "/Users/Me", want_status=401)

    auth = request("POST", "/Users/AuthenticateByName",
                   body=json.dumps({"Username": "demo", "Pw": "x"}).encode())
    token = auth["AccessToken"]
    assert auth["User"]["Id"] == lib.user_id and auth["ServerId"] == lib.server_id
    me = request("GET", "/Users/Me", token=token)
    assert me["Id"] == lib.user_id

    uid = lib.user_id
    movies_view, tv_view = lib.movies_view, lib.tv_view

    views = request("GET", "/Users/%s/Views" % uid, token=token)
    assert [v["CollectionType"] for v in views["Items"]] == ["movies", "tvshows"], views

    # A listing row carries every field convert.rs' movie_from_dto reads, with JSON types.
    page = request("GET", "/Users/%s/Items?ParentId=%s&Recursive=true&IncludeItemTypes=Movie"
                          "&Fields=%s&SortBy=SortName&SortOrder=Ascending&StartIndex=0&Limit=10"
                          % (uid, movies_view, urllib.parse.quote(LISTING_FIELDS)), token=token)
    assert page["TotalRecordCount"] == len(lib.movies)
    movie = page["Items"][0]
    for key in ("Id", "Name", "Type", "ProductionYear", "Overview", "RunTimeTicks",
                "OfficialRating", "CommunityRating", "PremiereDate", "UserData",
                "ImageTags", "BackdropImageTags", "MediaSources"):
        assert key in movie, (key, movie)
    assert movie["Type"] == "Movie" and isinstance(movie["UserData"]["Played"], bool)
    assert movie["RunTimeTicks"] > 0
    source = movie["MediaSources"][0]
    assert source["Id"] and source["MediaStreams"][0]["Type"] == "Video" \
        and source["MediaStreams"][0]["Codec"] in VIDEO_CODECS

    # Paging, unwatched filter, Limit=0 count probe (the A–Z rail), genre filter, NameStartsWith.
    paged = request("GET", "/Users/%s/Items?ParentId=%s&Recursive=true&IncludeItemTypes=Movie"
                          "&StartIndex=2&Limit=5&SortBy=SortName&SortOrder=Ascending"
                          % (uid, movies_view), token=token)
    assert len(paged["Items"]) == 5 and paged["Items"][0]["Id"] != movie["Id"]
    count = request("GET", "/Users/%s/Items?ParentId=%s&Recursive=true&IncludeItemTypes=Movie"
                          "&Limit=0" % (uid, movies_view), token=token)
    assert count["Items"] == [] and count["TotalRecordCount"] == len(lib.movies)
    unwatched = request("GET", "/Users/%s/Items?ParentId=%s&Recursive=true&IncludeItemTypes=Movie"
                              "&IsPlayed=false&Limit=0" % (uid, movies_view), token=token)
    assert 0 < unwatched["TotalRecordCount"] < len(lib.movies)
    names = sorted(lib.items[i]["Name"] for i in lib.movies)
    letter = names[0][0].upper()
    starts = request("GET", "/Users/%s/Items?ParentId=%s&Recursive=true&IncludeItemTypes=Movie"
                          "&NameStartsWith=%s&Limit=0" % (uid, movies_view, letter), token=token)
    assert starts["TotalRecordCount"] == len(lib.movies)  # every name starts with the same letter

    # Latest answers the bare array; the tv view collapses to series rows.
    latest = request("GET", "/Users/%s/Items/Latest?ParentId=%s&Limit=5"
                     % (uid, movies_view), token=token)
    assert isinstance(latest, list) and len(latest) == 5 and latest[0]["Type"] == "Movie"
    latest_tv = request("GET", "/Users/%s/Items/Latest?ParentId=%s&Limit=5"
                        % (uid, tv_view), token=token)
    assert latest_tv[0]["Type"] == "Series"

    # Detail: the Fields the DETAIL page asks for, plus the trickplay manifest on demand.
    detail = request("GET", "/Users/%s/Items/%s?Fields=%s"
                     % (uid, movie["Id"], urllib.parse.quote(DETAIL_FIELDS)), token=token)
    for key in ("Genres", "People", "Chapters", "Taglines", "ProductionLocations",
                "MediaSources", "Overview"):
        assert key in detail, (key, sorted(detail))
    assert detail["People"][0]["Type"] in ("Actor", "Director", "Writer")
    trickplay = request("GET", "/Users/%s/Items/%s?Fields=Trickplay" % (uid, movie["Id"]),
                        token=token)
    sheet = trickplay["Trickplay"]["320"]["0"]
    assert sheet["TileWidth"] == 10 and sheet["Interval"] == 10000

    series_id = lib.series[1]  # index 2 — a STARTED series
    seasons = request("GET", "/Shows/%s/Seasons?UserId=%s" % (series_id, uid), token=token)
    assert seasons["Items"][0]["Type"] == "Season" and seasons["Items"][0]["SeriesId"] == series_id
    episodes = request("GET", "/Shows/%s/Episodes?UserId=%s&SeasonId=%s"
                       % (series_id, uid, seasons["Items"][0]["Id"]), token=token)
    first_episode = episodes["Items"][0]
    assert first_episode["Type"] == "Episode" and first_episode["ParentIndexNumber"] == 1
    assert first_episode["SeriesId"] == series_id

    next_up = request("GET", "/Shows/NextUp?UserId=%s&Limit=10" % uid, token=token)
    assert next_up["Items"] and next_up["Items"][0]["Type"] == "Episode"
    started = request("GET", "/Shows/NextUp?UserId=%s&SeriesId=%s" % (uid, series_id), token=token)
    assert len(started["Items"]) == 1, "a started series has a next-up episode"
    unstarted = request("GET", "/Shows/NextUp?UserId=%s&SeriesId=%s" % (uid, lib.series[0]),
                        token=token)
    assert unstarted["Items"] == [], "a never-started series has none"

    similar = request("GET", "/Items/%s/Similar?UserId=%s&Limit=4" % (movie["Id"], uid),
                      token=token)
    assert len(similar["Items"]) == 4 and all(r["Type"] == "Movie" for r in similar["Items"])

    genres = request("GET", "/Genres?ParentId=%s&UserId=%s" % (movies_view, uid), token=token)
    assert genres["Items"][0]["Type"] == "Genre" and genres["Items"][0]["Id"]

    # Search: media page + people page.
    needle = lib.items[lib.movies[0]]["Name"][:4].lower()
    found = request("GET", "/Users/%s/Items?searchTerm=%s&Recursive=true"
                          "&IncludeItemTypes=Movie,Series,Episode,BoxSet&Limit=12"
                    % (uid, urllib.parse.quote(needle)), token=token)
    assert any(needle in it["Name"].lower() for it in found["Items"]), (needle, found)
    person = lib.people[lib.people_by_index[0]]
    people = request("GET", "/Persons?searchTerm=%s&UserId=%s&Limit=12"
                     % (urllib.parse.quote(person["Name"]), uid), token=token)
    assert people["Items"] and people["Items"][0]["Id"] == person["Id"]

    # A filmography query: person-scoped items page.
    filmography = request("GET", "/Users/%s/Items?Recursive=true&PersonIds=%s"
                                 "&IncludeItemTypes=Movie,Series&Fields=People&SortBy=SortName"
                          % (uid, person["Id"]), token=token)
    assert filmography["TotalRecordCount"] > 0

    resume = request("GET", "/Users/%s/Items/Resume?Limit=20&MediaTypes=Video" % uid, token=token)
    assert resume["Items"], "the seed patterns keep Resume non-empty"
    assert all(0 < it["UserData"]["PlaybackPositionTicks"] for it in resume["Items"])

    image = request("GET", "/Items/%s/Images/Primary?tag=%s&fillWidth=300&api_key=%s"
                    % (movie["Id"], movie["ImageTags"]["Primary"], token))
    assert image[:3] == b"\xff\xd8\xff", "not a JPEG"
    request("GET", "/Items/%s/Images/Primary?tag=x" % movie["Id"], want_status=401)

    playback = request("POST", "/Items/%s/PlaybackInfo?UserId=%s" % (movie["Id"], uid),
                       body=json.dumps({"DeviceProfile": {}}).encode(), token=token)
    assert playback["PlaySessionId"] and playback["MediaSources"][0]["Id"]
    assert "TranscodingUrl" not in playback["MediaSources"][0], "direct play is the verdict"

    # Playback reporting: open, progress, stop — and the stop COMMITS the resume point.
    session = {"ItemId": movie["Id"], "PlaySessionId": "ps-1", "CanSeek": True}
    request("POST", "/Sessions/Playing", body=json.dumps(
        dict(session, PositionTicks=60_000_000, IsPaused=False)).encode(), token=token,
        want_status=204)
    request("POST", "/Sessions/Playing/Progress", body=json.dumps(
        dict(session, PositionTicks=120_000_000, IsPaused=True)).encode(), token=token,
        want_status=204)
    request("POST", "/Sessions/Playing/Stopped", body=json.dumps(
        dict(session, PositionTicks=120_000_000, IsPaused=False)).encode(), token=token,
        want_status=204)
    resumed = request("GET", "/Users/%s/Items/Resume?Limit=50" % uid, token=token)
    assert any(it["Id"] == movie["Id"] and it["UserData"]["PlaybackPositionTicks"] == 120_000_000
               for it in resumed["Items"]), "Stopped did not commit the resume point"

    # View-state writes mutate the served state.
    request("POST", "/Users/%s/PlayedItems/%s" % (uid, movie["Id"]), token=token)
    played_row = request("GET", "/Users/%s/Items/%s" % (uid, movie["Id"]), token=token)
    assert played_row["UserData"]["Played"] is True
    request("DELETE", "/Users/%s/PlayedItems/%s" % (uid, movie["Id"]), token=token)
    request("POST", "/Users/%s/FavoriteItems/%s" % (uid, movie["Id"]), token=token)
    fav_row = request("GET", "/Users/%s/Items/%s" % (uid, movie["Id"]), token=token)
    assert fav_row["UserData"]["IsFavorite"] is True
    request("POST", "/Users/%s/Items/%s/UserData" % (uid, movie["Id"]),
            body=json.dumps({"PlaybackPositionTicks": 0}).encode(), token=token)
    cleared = request("GET", "/Users/%s/Items/%s" % (uid, movie["Id"]), token=token)
    assert cleared["UserData"]["PlaybackPositionTicks"] == 0

    request("DELETE", "/Videos/ActiveEncodings?playSessionId=ps-1&deviceId=dev", token=token,
            want_status=204)
    request("GET", "/MediaSegments/%s" % movie["Id"], token=token)

    # The static stream the player opens: the whole body, then one Range slice. Skipped by
    # name on a machine that has not run the demo library, which is the same machine that has
    # no clip to serve.
    clip = fallback_stream()
    if clip:
        stream_path = "/Videos/%s/stream?static=true&api_key=%s" % (movie["Id"], token)
        with urllib.request.urlopen(base + stream_path, timeout=10) as reply:
            assert reply.status == 200 and reply.headers["Accept-Ranges"] == "bytes"
            whole = reply.read()
        assert len(whole) == clip.stat().st_size, (len(whole), clip.stat().st_size)
        assert whole[:4] == b"\x00\x00\x00\x20" or whole[4:8] == b"ftyp", whole[:8]
        ranged = urllib.request.Request(base + stream_path, headers={"Range": "bytes=0-99"})
        with urllib.request.urlopen(ranged, timeout=10) as reply:
            assert reply.status == 206 and len(reply.read()) == 100, reply.status
            assert reply.headers["Content-Range"] == "bytes 0-99/%d" % len(whole)
    else:
        print("selftest: no stand-in clip on disk, the stream route was not exercised")

    # Quick Connect: initiate, poll twice (authorized on the 2nd), trade the secret.
    qc = request("POST", "/QuickConnect/Initiate", body=b"{}")
    assert qc["Secret"] and re.fullmatch(r"\d{6}", qc["Code"])
    state = request("GET", "/QuickConnect/Connect?secret=%s" % qc["Secret"])
    assert state["Authenticated"] is False
    state = request("GET", "/QuickConnect/Connect?secret=%s" % qc["Secret"])
    assert state["Authenticated"] is True
    qc_auth = request("POST", "/Users/AuthenticateWithQuickConnect",
                      body=json.dumps({"Secret": qc["Secret"]}).encode())
    assert qc_auth["AccessToken"] == token

    # Determinism: a second seed-7 world answers byte-identically where it matters.
    srv2, lib2 = serve(0, seed=7)
    assert lib2.user_id == lib.user_id and lib2.movies_view == lib.movies_view
    assert json.dumps(lib2.items[lib2.movies[0]], sort_keys=True) == \
        json.dumps(lib.items[lib.movies[0]], sort_keys=True)

    srv.shutdown()

    # The demo catalog's world (`tools/screenshots.py` records against it): the same routes, real
    # films, real artwork. Skipped by name where the derived cache is absent, so a fresh clone's
    # selftest stays offline.
    if catalog is not None or (demo_cache_dir() / "derived").is_dir():
        _selftest_catalog(DEMO_CATALOG if catalog is None else catalog)

    if failures:
        print("mock_jf: selftest FAILED:", file=sys.stderr)
        for failure in failures:
            print("  " + failure, file=sys.stderr)
        sys.exit(1)
    print("mock_jf: selftest OK (%d requests exercised)" % len(srv.requests))


def _selftest_catalog(catalog_path):
    """The catalog world through the same HTTP surface: ids derived from slugs, the deck in the
    catalog's order with the hero at its head, real artwork bytes, a box set's own order, and two
    constructions that agree — the determinism a capture grades."""
    import urllib.error
    import urllib.request

    srv, lib = serve(0, catalog=catalog_path)
    run(srv)
    base = "http://127.0.0.1:%d" % srv.server_address[1]
    token = lib.token
    auth = 'MediaBrowser Token="%s", Client="T", Device="D", DeviceId="dev", Version="1"' % token

    def get(path, raw=False):
        request = urllib.request.Request(base + path)
        request.add_header("Authorization", auth)
        with urllib.request.urlopen(request, timeout=10) as reply:
            data = reply.read()
            return (reply.headers.get("Content-Type"), data) if raw else json.loads(data)

    uid = lib.user_id
    cat = lib.catalog
    assert len(lib.movies) == len(cat["movies"]) and len(lib.series) == len(cat["shows"])
    for slug in (cat["hero"], "caminandes/1/2"):
        assert slug in lib.by_slug, slug
        assert len(lib.by_slug[slug]) == 32, "a Jellyfin GUID is 32 hex characters"

    resume = get("/Users/%s/Items/Resume?Limit=20" % uid)["Items"]
    assert resume and resume[0]["Id"] == lib.by_slug[cat["hero"]], "the hero heads the deck"
    assert all(0 < it["UserData"]["PlaybackPositionTicks"] < it["RunTimeTicks"] for it in resume)

    page = get("/Users/%s/Items?ParentId=%s&Recursive=true&IncludeItemTypes=Movie&Limit=0"
               % (uid, lib.movies_view))
    assert page["TotalRecordCount"] == len(cat["movies"])

    box = lib.items[lib.boxsets[0]]
    kids = get("/Users/%s/Items?ParentId=%s&SortBy=SortName&SortOrder=Ascending"
               % (uid, box["Id"]))["Items"]
    pinned = lib.pinned_rank(box["Id"])
    if pinned:
        assert [k["Id"] for k in kids] == [k for k, _ in
                                           sorted(pinned.items(), key=lambda kv: kv[1])]

    still = lib.by_slug["caminandes/1/2"]
    ctype, data = get("/Items/%s/Images/Primary?api_key=%s" % (still, token), raw=True)
    assert ctype == "image/jpeg" and data[:2] == b"\xff\xd8", (ctype, data[:4])
    art_id = lib.by_slug[cat["hero"]]
    ctype, data = get("/Items/%s/Images/Backdrop/0?api_key=%s" % (art_id, token), raw=True)
    assert ctype == "image/jpeg" and data[:2] == b"\xff\xd8", (ctype, data[:4])
    assert len(data) > 10_000, "a real backdrop, not the placeholder"

    # An item that carries its own film must serve THAT film. The scenes share one stand-in, so
    # serving it to every item made the player scene reach the end of a three-minute clip before
    # the instant it pauses at (7:26 of the catalog movie).
    film_id = lib.by_slug["sintel"]
    film = lib.streams.get(film_id)
    assert film is not None, "the catalog film is cached but no stream was registered for it"
    request = urllib.request.Request(
        base + "/Videos/%s/stream?static=true&api_key=%s" % (film_id, token),
        headers={"Range": "bytes=0-99"})
    with urllib.request.urlopen(request, timeout=10) as reply:
        assert reply.status == 206 and len(reply.read()) == 100, reply.status
        assert reply.headers["Content-Range"] == "bytes 0-99/%d" % film.stat().st_size
    assert film.stat().st_size > 60 * 1024 * 1024, "the film itself, not a derived stand-in"

    srv.shutdown()

    srv2, lib2 = serve(0, catalog=catalog_path)
    assert lib2.user_id == lib.user_id and lib2.movies_view == lib.movies_view
    assert json.dumps(sorted(lib2.by_slug.items())) == json.dumps(sorted(lib.by_slug.items()))
    assert json.dumps(lib2.items[lib2.by_slug[cat["hero"]]], sort_keys=True) == \
        json.dumps(lib.items[lib.by_slug[cat["hero"]]], sort_keys=True)
    srv2.server_close()
    print("mock_jf: catalog selftest OK (%d movies, %d shows, %d box sets)"
          % (len(lib.movies), len(lib.series), len(lib.boxsets)))


def main():
    ap = argparse.ArgumentParser(
        description=__doc__.split("\n\n")[0],
        epilog=__doc__[__doc__.index("Usage:"):],
        formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--port", type=int, default=8920)
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--movies", type=int, default=48)
    ap.add_argument("--shows", type=int, default=6)
    ap.add_argument("--catalog", nargs="?", const=True, default=None, metavar="PATH",
                    help="serve the demo library (tests/demo_library/catalog.json, or PATH)")
    ap.add_argument("--catalog-cache", metavar="DIR",
                    help="the demo artwork cache (default: ~/.cache/butaca-demo)")
    ap.add_argument("--hero", help="the film the home deck pins (default: the catalog's `hero`)")
    ap.add_argument("--selftest", action="store_true")
    a = ap.parse_args()
    if a.selftest:
        selftest(a.catalog)
        return
    srv, lib = serve(a.port, seed=a.seed, host=a.host, movies=a.movies, shows=a.shows,
                     catalog=a.catalog, catalog_cache=a.catalog_cache, hero=a.hero)
    if a.catalog is not None:
        print("mock_jf: serving the demo library on http://%s:%d (user id %s, %d movies, "
              "%d series, %d box sets)"
              % (a.host, srv.server_address[1], lib.user_id, len(lib.movies), len(lib.series),
                 len(lib.boxsets)), flush=True)
    else:
        print("mock_jf: serving seed=%d on http://%s:%d (user id %s, %d movies, %d series)"
              % (a.seed, a.host, srv.server_address[1], lib.user_id, len(lib.movies),
                 len(lib.series)), flush=True)
    try:
        srv.serve_forever(poll_interval=0.5)
    except KeyboardInterrupt:
        pass
    finally:
        srv.shutdown()
        with srv.lock:
            if srv.unknown:
                print("mock_jf: unknown paths seen: " + ", ".join(sorted(set(srv.unknown))),
                      file=sys.stderr)
            if srv.write_log:
                print("mock_jf: writes seen: %d" % len(srv.write_log), file=sys.stderr)


if __name__ == "__main__":
    main()
