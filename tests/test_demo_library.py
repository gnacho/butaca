"""The demo library and the screenshot manifest: the contracts `make screenshots` relies on.

The first half needs nothing but this checkout - the manifests, the scene manifest and the
driver's own spec parsing. The second half serves the catalog itself, which needs the derived
artwork cache (`make demo-library`, ~390 MB of downloads); it is skipped, loudly, where the cache
is absent, so a fresh clone's `make check` stays offline.
"""
import contextlib
import datetime
import importlib.util
import io
import json
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
import unittest.mock
import urllib.error
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tests"))

import mock_jf  # noqa: E402

# tools/demo_library.py shares its name with the tests/demo_library package, so it is loaded by
# path under another name rather than through sys.path.
_spec = importlib.util.spec_from_file_location("demo_library_tool", ROOT / "tools" / "demo_library.py")
tool = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(tool)
_spec = importlib.util.spec_from_file_location("screenshots_tool", ROOT / "tools" / "screenshots.py")
screenshots = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(screenshots)

SCENES = ROOT / "tests" / "screenshots" / "scenes.json"
CATALOG = ROOT / "tests" / "demo_library" / "catalog.json"
RUST = ROOT / "rust-modules" / "src"
# Armed by tools/screenshots.py on every scene, not by the manifest.
DRIVER_TRIGGERS = {"stillclock"}

HAVE_CACHE = (mock_jf.demo_cache_dir() / "derived").is_dir() and shutil.which("ffmpeg") and shutil.which("ffprobe")
# The identity every request to the mock carries: the client's own carrier shape (its token is
# accepted, never checked - mock_jf's module doc says why).
AUTH = 'MediaBrowser Token="%s", Client="T", Device="D", DeviceId="dev", Version="1"'


class Manifests(unittest.TestCase):
    def test_the_manifests_validate(self):
        assets, catalog = tool.load()
        self.assertTrue(tool.check(assets, catalog))

    def test_every_asset_is_pinned_and_openly_licensed(self):
        assets, _ = tool.load()
        for aid, a in assets.items():
            with self.subTest(asset=aid):
                self.assertRegex(a["sha256"], r"^[0-9a-f]{64}$")
                self.assertGreater(a["bytes"], 0)
                self.assertTrue(a["url"].startswith("https://"), a["url"])
                self.assertTrue(a["licence"].startswith(("CC BY", "CC0", "Public domain")), a["licence"])

    def test_the_hero_and_its_alternatives_are_distinct_films(self):
        _, catalog = tool.load()
        films = {m["id"] for m in catalog["movies"]}
        pool = [catalog["hero"], *catalog["hero_alternatives"]]
        self.assertEqual(len(pool), len(set(pool)))
        self.assertLessEqual(set(pool), films)

    def test_check_refuses_an_unknown_reference(self):
        assets, catalog = tool.load()
        broken = dict(catalog, watched=[*catalog["watched"], "no-such-film"])
        with self.assertRaises(AssertionError):
            tool.check(assets, broken)

    def test_every_hero_candidate_has_a_logo_cut_from_its_own_poster(self):
        # The home hero draws a film's clearLogo; a candidate without one falls back to text.
        _, catalog = tool.load()
        films = {m["id"]: m for m in catalog["movies"]}
        for film in (catalog["hero"], *catalog["hero_alternatives"]):
            with self.subTest(hero=film):
                self.assertIn("logo", films[film])
                self.assertEqual(films[film]["logo"]["asset"], films[film]["poster"]["asset"])

    def test_check_refuses_a_logo_cut_from_another_films_poster(self):
        assets, catalog = tool.load()
        movies = [dict(m) for m in catalog["movies"]]
        donor = next(m for m in movies if "logo" in m)
        other = next(m for m in movies if m["poster"]["asset"] != donor["poster"]["asset"])
        other["logo"] = donor["logo"]
        with self.assertRaises(AssertionError):
            tool.check(assets, dict(catalog, movies=movies))

    def test_check_refuses_a_stand_in_anywhere_but_an_episode(self):
        assets, catalog = tool.load()
        movies = [dict(m) for m in catalog["movies"]]
        movies[0]["stand_in"] = True
        with self.assertRaises(AssertionError):
            tool.check(assets, dict(catalog, movies=movies))

    def test_check_refuses_a_share_alike_licence(self):
        assets, catalog = tool.load()
        aid = next(iter(assets))
        broken = dict(assets, **{aid: dict(assets[aid], licence="CC BY-SA 4.0")})
        with self.assertRaises(AssertionError):
            tool.check(broken, catalog)

    def test_check_refuses_a_collection_order_that_is_not_the_whole_collection(self):
        assets, catalog = tool.load()
        order = dict(catalog["collection_order"])
        order["Blender Open Movies"] = order["Blender Open Movies"][:-1]
        with self.assertRaises(AssertionError):
            tool.check(assets, dict(catalog, collection_order=order))


class Credits(unittest.TestCase):
    """CREDITS.md is written by the same command that writes the images, so it cannot lag them."""

    def test_the_committed_credits_are_what_the_manifests_say(self):
        assets, catalog = tool.load()
        with tempfile.TemporaryDirectory() as d:
            dst = pathlib.Path(d) / "CREDITS.md"
            tool.credits(assets, catalog, dst)
            self.assertEqual(dst.read_text(), tool.CREDITS.read_text(),
                             "docs/screenshots/CREDITS.md is stale: run `make screenshots`")

    def test_the_committed_site_credits_page_is_what_the_manifests_say(self):
        assets, catalog = tool.load()
        with tempfile.TemporaryDirectory() as d:
            dst = pathlib.Path(d) / "credits.html"
            tool.site_credits(assets, catalog, dst)
            self.assertEqual(dst.read_text(), tool.SITE_CREDITS.read_text(),
                             "site/credits.html is stale: run `python3 tools/demo_library.py site-credits`")

    def test_the_site_credits_page_credits_every_asset_and_links_its_licence(self):
        assets, catalog = tool.load()
        page = tool.SITE_CREDITS.read_text()
        for aid, a in assets.items():
            self.assertIn(f'href="{a["source_page"]}"', page, f"{aid}: no source link on the credits page")
        for name, url in tool.LICENCE_TEXTS.items():
            if any(a["licence"] == name for a in assets.values()):
                self.assertIn(f'<a href="{url}" rel="license">{name}</a>', page)
        for m in catalog["movies"] + catalog["shows"]:
            self.assertIn(f'id="{m["id"]}"', page)

    def test_the_landing_page_footer_links_the_credits(self):
        self.assertIn('href="credits.html"', (ROOT / "site" / "index.html").read_text())
        self.assertIn("cp site/credits.html _site/credits.html",
                      (ROOT / ".github" / "workflows" / "pages.yml").read_text())

    def test_a_screenshot_run_writes_the_credits_beside_its_images(self):
        with tempfile.TemporaryDirectory() as d:
            screenshots.write_credits(pathlib.Path(d))
            self.assertEqual((pathlib.Path(d) / "CREDITS.md").read_text(), tool.CREDITS.read_text())


class RenderSet(unittest.TestCase):
    """A run replaces the figure set whole, or leaves it as it was."""

    JOBS = [({"name": "a"}, None, [{"file": "a.jpg"}]),
            ({"name": "b"}, None, [{"file": "b.jpg", "dest": "site"}])]

    @staticmethod
    def write(scene, hero, outputs, stage):
        o = outputs[0]
        (stage / o.get("dest", "docs") / o["file"]).write_bytes(b"new")

    def test_a_failure_of_any_kind_leaves_every_destination_as_it_was(self):
        def render(scene, hero, outputs, stage):
            self.write(scene, hero, outputs, stage)
            if scene["name"] == "b":
                raise subprocess.CalledProcessError(1, ["ffmpeg"])
        with tempfile.TemporaryDirectory() as d:
            docs, site = pathlib.Path(d) / "docs", pathlib.Path(d) / "site"
            docs.mkdir()
            (docs / "a.jpg").write_bytes(b"old")
            with contextlib.redirect_stderr(io.StringIO()):
                failed = screenshots.render_set(self.JOBS, render, {"docs": docs, "site": site})
            self.assertEqual(failed, ["b"])
            self.assertEqual(sorted(p.name for p in docs.iterdir()), ["a.jpg"])
            self.assertEqual((docs / "a.jpg").read_bytes(), b"old")
            self.assertFalse(site.exists())

    def test_a_clean_run_moves_each_output_to_its_destination_and_the_credits_beside_the_docs(self):
        with tempfile.TemporaryDirectory() as d:
            docs, site = pathlib.Path(d) / "docs", pathlib.Path(d) / "site"
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(screenshots.render_set(self.JOBS, self.write, {"docs": docs, "site": site}), [])
            self.assertEqual(sorted(p.name for p in docs.iterdir()), ["CREDITS.md", "a.jpg"])
            self.assertEqual(sorted(p.name for p in site.iterdir()), ["b.jpg"])


class OutputSpec(unittest.TestCase):
    """`render_scale` and `crop`: a crop is in canvas coordinates, whatever the scale."""

    CANVAS = (1920, 1080)

    def spec(self, out, **scene):
        return screenshots.output_spec(dict({"name": "t"}, **scene), out, self.CANVAS)

    def test_an_uncropped_output_is_the_whole_canvas_at_the_scale(self):
        self.assertEqual(self.spec({"file": "a.jpg"}), ("docs", "a.jpg", (0, 0, 1920, 1080), (1920, 1080), 2))
        self.assertEqual(self.spec({"file": "a.jpg"}, render_scale=3)[2:4], ((0, 0, 5760, 3240), (5760, 3240)))

    def test_a_crop_scales_with_the_render_and_keeps_its_size_unless_one_is_named(self):
        dest, _, crop, size, q = self.spec({"file": "g.jpg", "dest": "site", "crop": [680, 0, 880, 160],
                                            "quality": 4}, render_scale=3)
        self.assertEqual((dest, crop, size, q), ("site", (2040, 0, 2640, 480), (2640, 480), 4))
        self.assertEqual(self.spec({"file": "g.jpg", "crop": [680, 0, 880, 160], "size": "1320x240"},
                                   render_scale=3)[3], (1320, 240))

    def test_a_fractional_crop_rounds_its_edges_not_its_size(self):
        # 846.667 canvas px at 3x is 2540 px: the edges round, so the size is what they enclose.
        _, _, crop, size, _ = self.spec({"file": "t.jpg", "crop": [100.333, 20, 846.667, 513.333]},
                                        render_scale=3)
        self.assertEqual(crop, (301, 60, 2540, 1540))
        self.assertEqual(size, (2540, 1540))

    def test_what_the_driver_cannot_honour_is_refused(self):
        bad = [
            ({"file": "a.jpg"}, {"render_scale": 5}),
            ({"file": "a.jpg"}, {"render_scale": 2.5}),
            ({"file": "a.jpg"}, {"render_scale": True}),
            ({"file": "a.jpg", "crop": [1800, 0, 200, 100]}, {}),
            ({"file": "a.jpg", "crop": [0, 0, 0, 100]}, {}),
            ({"file": "a.jpg", "crop": [-1, 0, 10, 10]}, {}),
            ({"file": "a.jpg", "crop": [0, 0, "10", 10]}, {}),
            ({"file": "a.jpg", "dest": "elsewhere"}, {}),
            ({"file": "../a.jpg"}, {}),
            ({"file": "a.png"}, {}),
            ({"file": "a.jpg", "quality": 0}, {}),
            ({"file": "a.jpg", "crop": [0, 0, 880, 160], "size": "880x200"}, {}),
        ]
        for out, scene in bad:
            with self.subTest(out=out, scene=scene), self.assertRaises(ValueError):
                self.spec(out, **scene)

    def test_every_manifest_output_resolves(self):
        manifest = json.loads(SCENES.read_text())
        canvas = screenshots.size_of(manifest["canvas"])
        for s in manifest["scenes"]:
            for o in s["outputs"]:
                with self.subTest(scene=s["name"], file=o["file"]):
                    screenshots.resolve_output(s, o, canvas)


class ItemRefs(unittest.TestCase):
    """`@<slug>` in a trigger or an expect line is the catalog item's id, resolved per run."""

    BY_SLUG = {"sintel": "a" * 32, "caminandes/1/2": "b" * 32}

    def test_a_trigger_value_resolves_to_the_id(self):
        self.assertEqual(screenshots.resolve_refs("sintel", self.BY_SLUG), "sintel")
        self.assertEqual(screenshots.resolve_refs("@sintel", self.BY_SLUG), "a" * 32)
        self.assertEqual(screenshots.resolve_refs("@caminandes/1/2", self.BY_SLUG), "b" * 32)

    def test_an_expect_line_resolves_every_reference_it_carries(self):
        line = "detail: sid=0 rk=@sintel next=S1E3 @caminandes/1/2"
        self.assertEqual(screenshots.resolve_refs(line, self.BY_SLUG),
                         "detail: sid=0 rk=%s next=S1E3 %s" % ("a" * 32, "b" * 32))

    def test_an_unknown_reference_is_refused(self):
        with self.assertRaises(ValueError):
            screenshots.resolve_refs("@no-such-film", self.BY_SLUG)

    def test_every_reference_in_the_manifest_names_the_catalog(self):
        if not HAVE_CACHE:
            self.skipTest("no derived demo cache (make demo-library)")
        by_slug = mock_jf.CatalogLibrary(CATALOG).by_slug
        manifest = json.loads(SCENES.read_text())
        for s in manifest["scenes"]:
            for value in s.get("triggers", {}).values():
                with self.subTest(scene=s["name"], value=value):
                    screenshots.resolve_refs(str(value), by_slug)
            for line in s.get("expect", []):
                with self.subTest(scene=s["name"], line=line):
                    screenshots.resolve_refs(line, by_slug)


class CardSpec(unittest.TestCase):
    """A `card` output is composed around another output of its scene, never cut from the capture."""

    SCENE = {"name": "home", "outputs": [{"file": "home.jpg", "size": "1600x900"},
                                         {"file": "og-card.jpg", "dest": "site", "card": "home.jpg"}]}

    def test_a_card_resolves_to_its_source_output(self):
        dest, file, source = screenshots.card_spec(self.SCENE, self.SCENE["outputs"][1])
        self.assertEqual((dest, file, source["file"]), ("site", "og-card.jpg", "home.jpg"))

    def test_a_card_without_a_source_or_with_pixel_keys_is_refused(self):
        for out in ({"file": "og.jpg", "card": "missing.jpg"},
                    {"file": "og.jpg", "card": "og.jpg"},
                    {"file": "og.jpg", "card": "home.jpg", "crop": [0, 0, 10, 10]},
                    {"file": "og.png", "card": "home.jpg"}):
            scene = dict(self.SCENE, outputs=[*self.SCENE["outputs"], out])
            with self.subTest(out=out), self.assertRaises(ValueError):
                screenshots.card_spec(scene, out)

    def test_the_manifest_renders_the_link_preview_from_the_home_figure(self):
        manifest = json.loads(SCENES.read_text())
        cards = [(s["name"], o) for s in manifest["scenes"] for o in s["outputs"] if "card" in o]
        self.assertEqual(cards, [("home", {"file": "og-card.jpg", "dest": "site", "card": "home.jpg"})])


class Fetch(unittest.TestCase):
    def test_a_failed_download_leaves_no_partial_file(self):
        aid, asset = next(iter(tool.load()[0].items()))
        with tempfile.TemporaryDirectory() as d, \
                unittest.mock.patch.object(tool, "cache_dir", return_value=pathlib.Path(d)), \
                unittest.mock.patch.object(tool.urllib.request, "urlopen", side_effect=OSError("offline")), \
                contextlib.redirect_stdout(io.StringIO()):
            with self.assertRaises(OSError):
                tool.fetch({aid: asset})
            self.assertEqual(list((pathlib.Path(d) / "src").iterdir()), [])


class Chapters(unittest.TestCase):
    chapters = staticmethod(mock_jf.CatalogLibrary.chapters)

    def test_chapters_carry_the_catalog_titles_as_tick_offsets(self):
        rows = self.chapters([{"start": "0:00", "title": "A"}, {"start": "1:41", "title": "B"}], 300_000, "x")
        self.assertEqual([(r["Name"], r["StartPositionTicks"]) for r in rows],
                         [("A", 0), ("B", 1_010_000_000)])

    def test_malformed_chapters_are_refused(self):
        for marks in ([{"start": "0:05", "title": "late"}],
                      [{"start": "0:00", "title": "a"}, {"start": "0:00", "title": "b"}],
                      [{"start": "0:00", "title": "a"}, {"start": "9:00", "title": "past the end"}]):
            with self.subTest(marks=marks), self.assertRaises(ValueError):
                self.chapters(marks, 300_000, "x")

    def test_the_catalog_chapters_parse(self):
        _, catalog = tool.load()
        for m in catalog["movies"]:
            if m.get("chapters"):
                self.assertEqual(len(self.chapters(m["chapters"], m["minutes"] * 60_000, m["id"])),
                                 len(m["chapters"]))


class SceneManifest(unittest.TestCase):
    def setUp(self):
        self.manifest = json.loads(SCENES.read_text())
        self.scenes = self.manifest["scenes"]

    def test_names_and_outputs_are_unique(self):
        names = [s["name"] for s in self.scenes]
        files = [o["file"] for s in self.scenes for o in s["outputs"]]
        self.assertEqual(len(names), len(set(names)))
        self.assertEqual(len(files), len(set(files)))

    def test_every_documented_figure_has_a_scene(self):
        files = {o["file"] for s in self.scenes for o in s["outputs"]}
        readme = {"home.jpg", "library.jpg", "search.jpg", "player.jpg"}
        ux = {f"ux-{n}.jpg" for n in ("home-hero", "home-shelves", "signin", "library-grid", "library-sort",
                                      "search", "item-menu", "account-menu", "detail", "failure")}
        self.assertLessEqual(readme | ux, files)

    def test_sizes_and_the_canvas_parse(self):
        self.assertRegex(self.manifest["canvas"], r"^\d+x\d+$")
        for s in self.scenes:
            for o in s["outputs"]:
                if "size" in o:
                    self.assertRegex(o["size"], r"^\d+x\d+$", s["name"])
                self.assertTrue(o["file"].endswith(".jpg"), o["file"])

    def test_the_site_close_ups_are_all_rendered_and_supersampled(self):
        site = {o["file"]: s for s in self.scenes for o in s["outputs"]
                if o.get("dest") == "site" and "card" not in o}
        self.assertEqual(set(site), {"closeup-glass.jpg", "closeup-glass-narrow.jpg",
                                     "closeup-tiles.jpg", "closeup-player.jpg"})
        for file, s in site.items():
            with self.subTest(file=file):
                self.assertGreaterEqual(s.get("render_scale", 1), 3)

    def test_every_scene_names_its_state_and_a_log_line_proving_it(self):
        for s in self.scenes:
            with self.subTest(scene=s["name"]):
                self.assertTrue(s.get("state"))
                self.assertTrue(s.get("expect"))

    def test_a_loosened_bound_is_explained(self):
        for s in self.scenes:
            if s.get("free_regions") or "max_delta" in s:
                self.assertTrue(s.get("tolerance_reason"), s["name"])
            for x, y, w, h in s.get("free_regions", []):
                self.assertTrue(w > 0 and h > 0 and x >= 0 and y >= 0, s["name"])

    def test_every_trigger_is_one_the_app_reads(self):
        read = set()
        for path in RUST.rglob("*.rs"):
            read |= set(re.findall(r'dev::(?:read|flag)\("([a-z0-9_]+)"\)', path.read_text()))
            read |= set(re.findall(r'\b(?:read|flag)\("([a-z0-9_]+)"\)', path.read_text()))
        for s in self.scenes:
            for name in s.get("triggers", {}):
                with self.subTest(scene=s["name"], trigger=name):
                    self.assertIn(name, read)
                    self.assertNotIn(name, DRIVER_TRIGGERS, "the driver owns this trigger")

    def test_exactly_one_scene_renders_the_hero_variants(self):
        self.assertEqual(sum(1 for s in self.scenes if s.get("hero_variants")), 1)


@unittest.skipUnless(HAVE_CACHE, "no derived demo cache (make demo-library) or no ffmpeg/ffprobe")
class Catalog(unittest.TestCase):
    """The demo library on the Jellyfin wire: the world `make screenshots` records against."""

    @classmethod
    def setUpClass(cls):
        cls.srv, cls.lib = mock_jf.serve(0, catalog=CATALOG)
        mock_jf.run(cls.srv)
        cls.base = "http://127.0.0.1:%d" % cls.srv.server_address[1]
        cls.catalog = json.loads(CATALOG.read_text())

    @classmethod
    def tearDownClass(cls):
        cls.srv.shutdown()
        cls.srv.server_close()

    def get(self, path, raw=False):
        request = urllib.request.Request(self.base + path)
        request.add_header("Authorization", AUTH % self.lib.token)
        with urllib.request.urlopen(request, timeout=10) as reply:
            data = reply.read()
            return (reply.headers.get("Content-Type"), data) if raw else json.loads(data)

    def test_ids_are_derived_from_the_catalog_slugs(self):
        for rec in self.catalog["movies"] + self.catalog["shows"]:
            with self.subTest(item=rec["id"]):
                item_id = self.lib.by_slug[rec["id"]]
                self.assertRegex(item_id, r"^[0-9a-f]{32}$")
                self.assertEqual(self.lib.items[item_id]["Name"], rec["title"])

    def test_every_episode_has_an_id_of_its_own(self):
        slug = "caminandes/1/2"
        self.assertRegex(self.lib.by_slug[slug], r"^[0-9a-f]{32}$")
        self.assertNotEqual(self.lib.by_slug[slug], self.lib.by_slug["caminandes"])
        self.assertEqual(self.lib.items[self.lib.by_slug[slug]]["Name"], "Gran Dillama")

    def test_a_second_construction_answers_the_same_ids(self):
        other = mock_jf.CatalogLibrary(CATALOG)
        self.assertEqual(other.by_slug, self.lib.by_slug)

    def test_the_hero_heads_continue_watching_with_progress(self):
        deck = self.get("/Users/%s/Items/Resume?Limit=20&MediaTypes=Video" % self.lib.user_id)["Items"]
        head = deck[0]
        self.assertEqual(head["Id"], self.lib.by_slug[self.catalog["hero"]])
        self.assertGreater(head["UserData"]["PlaybackPositionTicks"], 0)
        self.assertLess(head["UserData"]["PlaybackPositionTicks"], head["RunTimeTicks"])

    def test_the_deck_is_the_catalog_continue_watching_in_order(self):
        deck = self.get("/Users/%s/Items/Resume?Limit=20" % self.lib.user_id)["Items"]
        self.assertEqual([it["Id"] for it in deck],
                         [self.lib.by_slug[c["item"]] for c in self.catalog["continue_watching"]])

    def test_hero_swaps_the_head_and_keeps_the_rest_in_order(self):
        for film in self.catalog["hero_alternatives"]:
            with self.subTest(hero=film):
                lib = mock_jf.CatalogLibrary(CATALOG, hero=film)
                pool = [lib.items[i] for i in lib.movies]
                for i in lib.series:
                    pool += lib.series_episodes(lib.items[i])
                deck = [it for it in pool if 0 < it["UserData"]["PlaybackPositionTicks"]
                        and not it["UserData"]["Played"]]
                deck.sort(key=lambda it: it["UserData"]["LastPlayedDate"] or "", reverse=True)
                self.assertEqual(deck[0]["Id"], lib.by_slug[film])
                rest = [it["Id"] for it in deck[1:]]
                want = [lib.by_slug[c["item"]] for c in self.catalog["continue_watching"]
                        if c["item"] != film]
                self.assertEqual(rest, want)

    def test_a_watched_film_is_played_and_an_unwatched_one_is_not(self):
        for slug in self.catalog["watched"]:
            with self.subTest(slug=slug):
                row = self.get("/Users/%s/Items/%s" % (self.lib.user_id, self.lib.by_slug[slug]))
                self.assertTrue(row["UserData"]["Played"])
        row = self.get("/Users/%s/Items/%s" % (self.lib.user_id, self.lib.by_slug["sprite-fright"]))
        self.assertFalse(row["UserData"]["Played"])

    def test_recently_added_follows_the_catalog_added_order(self):
        latest = self.get("/Users/%s/Items/Latest?ParentId=%s&Limit=5"
                          % (self.lib.user_id, self.lib.movies_view))
        want = [self.lib.by_slug[slug] for slug in self.catalog["added_order"]
                if slug in self.lib.by_slug and self.lib.items[self.lib.by_slug[slug]]["Type"] == "Movie"]
        self.assertEqual([it["Id"] for it in latest], want[:5])

    def test_a_series_says_how_many_episodes_are_left(self):
        row = self.get("/Users/%s/Items/%s" % (self.lib.user_id, self.lib.by_slug["caminandes"]))
        show = next(s for s in self.catalog["shows"] if s["id"] == "caminandes")
        episodes = [e for season in show["seasons"] for e in season["episodes"]]
        watched = {slug for slug in self.catalog["watched"] if slug.startswith("caminandes/")}
        left = sum(1 for e in episodes
                   if "caminandes/1/%d" % e["index"] not in watched)
        self.assertEqual(row["UserData"]["UnplayedItemCount"], left)
        self.assertEqual(row["UserData"]["Played"], left == 0)

    def test_an_episode_carries_its_series_and_its_catalog_minutes(self):
        row = self.get("/Users/%s/Items/%s" % (self.lib.user_id, self.lib.by_slug["caminandes/1/2"]))
        self.assertEqual(row["Type"], "Episode")
        self.assertEqual(row["SeriesId"], self.lib.by_slug["caminandes"])
        self.assertEqual(row["IndexNumber"], 2)
        self.assertEqual(row["ParentIndexNumber"], 1)
        self.assertEqual(row["RunTimeTicks"], 3 * mock_jf.TICKS_PER_MINUTE)

    def test_a_collection_lists_its_members_in_the_pinned_order(self):
        pinned = self.catalog["collection_order"]["Blender Open Movies"]
        box = next(i for i in self.lib.boxsets
                   if self.lib.items[i]["Name"] == "Blender Open Movies")
        kids = self.get("/Users/%s/Items?ParentId=%s&SortBy=SortName&SortOrder=Ascending"
                        % (self.lib.user_id, box))["Items"]
        self.assertEqual([k["Id"] for k in kids], [self.lib.by_slug[slug] for slug in pinned])

    def test_a_collection_without_a_pinned_order_lists_its_members_in_the_asked_order(self):
        name = "Silent Classics"
        box = next(i for i in self.lib.boxsets if self.lib.items[i]["Name"] == name)
        kids = self.get("/Users/%s/Items?ParentId=%s&SortBy=SortName&SortOrder=Ascending"
                        % (self.lib.user_id, box))["Items"]
        want = {m["id"] for m in self.catalog["movies"] if name in m.get("collections", [])}
        self.assertEqual({self.lib.items[k["Id"]]["Name"] for k in kids},
                         {m["title"] for m in self.catalog["movies"] if m["id"] in want})
        names = [self.lib.items[k["Id"]]["Name"] for k in kids]
        self.assertEqual(names, sorted(names, key=str.lower))

    def test_artwork_is_the_real_derived_file(self):
        for slug, kind, ctype in (("sintel", "Primary", "image/jpeg"),
                                  ("sintel", "Backdrop", "image/jpeg"),
                                  ("caminandes/1/2", "Primary", "image/jpeg")):
            with self.subTest(slug=slug, kind=kind):
                path = "/Items/%s/Images/%s" % (self.lib.by_slug[slug], kind)
                got_type, data = self.get(path + "?api_key=%s" % self.lib.token, raw=True)
                self.assertEqual(got_type, ctype)
                self.assertEqual(data[:2], b"\xff\xd8")
                self.assertGreater(len(data), 10_000, "the placeholder, not the derived artwork")

    def test_a_film_without_art_names_no_backdrop(self):
        bare = next(m["id"] for m in self.catalog["movies"] if "art" not in m)
        row = self.get("/Users/%s/Items/%s" % (self.lib.user_id, self.lib.by_slug[bare]))
        self.assertEqual(row["BackdropImageTags"], [])
        self.assertTrue(row["ImageTags"]["Primary"], "its poster is still there")

    def test_a_missing_image_and_an_unknown_item_are_404s(self):
        path = "/Items/%s/Images/Primary?api_key=%s"
        with self.assertRaises(urllib.error.HTTPError) as caught:
            self.get(path % ("0" * 32, self.lib.token), raw=True)
        self.assertEqual(caught.exception.code, 404)
        unknown_kind = "/Items/%s/Images/Screenshot?api_key=%s" % (self.lib.by_slug["sintel"],
                                                                  self.lib.token)
        with self.assertRaises(urllib.error.HTTPError) as caught:
            self.get(unknown_kind, raw=True)
        self.assertEqual(caught.exception.code, 404)

    def test_search_answers_media_and_people(self):
        media = self.get("/Users/%s/Items?searchTerm=camin&Recursive=true"
                         "&IncludeItemTypes=Movie,Series,Episode,BoxSet&Limit=12"
                         % self.lib.user_id)["Items"]
        self.assertIn(self.lib.by_slug["caminandes"], [it["Id"] for it in media])
        people = self.get("/Persons?searchTerm=pablo&UserId=%s&Limit=12"
                          % self.lib.user_id)["Items"]
        self.assertEqual([p["Name"] for p in people], ["Pablo Vazquez"])

    def test_the_player_film_has_its_catalog_chapters(self):
        row = self.get("/Users/%s/Items/%s" % (self.lib.user_id, self.lib.by_slug["sintel"]))
        self.assertEqual([c["Name"] for c in row["Chapters"]][:2], ["Snowbound", "The Shaman's Hut"])
        self.assertEqual(row["Chapters"][0]["StartPositionTicks"], 0)
        self.assertEqual(row["RunTimeTicks"], 15 * mock_jf.TICKS_PER_MINUTE)

    def test_two_libraries_serve_the_same_bytes(self):
        other, _ = mock_jf.serve(0, catalog=CATALOG)
        mock_jf.run(other)
        try:
            other_base = "http://127.0.0.1:%d" % other.server_address[1]
            for path in ("/Users/%s/Items/Resume?Limit=20" % self.lib.user_id,
                         "/Users/%s/Items?ParentId=%s&IncludeItemTypes=Movie&Limit=5"
                         % (self.lib.user_id, self.lib.movies_view),
                         "/Users/%s/Items/%s" % (self.lib.user_id, self.lib.by_slug["sintel"])):
                with self.subTest(path=path):
                    def fetch(base):
                        request = urllib.request.Request(base + path)
                        request.add_header("Authorization", AUTH % self.lib.token)
                        with urllib.request.urlopen(request, timeout=10) as reply:
                            return reply.read()
                    self.assertEqual(fetch(self.base), fetch(other_base))
        finally:
            other.shutdown()
            other.server_close()

    def test_nothing_reads_the_wall_clock(self):
        now = self.catalog["now"]
        stamp_of = lambda text: datetime.datetime.strptime(
            text, "%Y-%m-%dT%H:%M:%S.0000000Z").replace(tzinfo=datetime.timezone.utc).timestamp()
        for it in self.lib.items.values():
            if it.get("DateCreated"):
                self.assertLessEqual(stamp_of(it["DateCreated"]), now, it["Name"])
            played = it["UserData"].get("LastPlayedDate")
            if played:
                self.assertLessEqual(stamp_of(played), now, it["Name"])

    def test_an_unknown_hero_is_refused(self):
        with self.assertRaises(ValueError):
            mock_jf.CatalogLibrary(CATALOG, hero="no-such-film")


if __name__ == "__main__":
    if not HAVE_CACHE:
        print("test_demo_library: the Catalog cases are SKIPPED — no derived demo cache "
              f"({mock_jf.demo_cache_dir()}); `make demo-library` builds it", file=sys.stderr)
    unittest.main(verbosity=1)
