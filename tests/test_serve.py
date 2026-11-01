"""nlab serve-site: build only when needed, then serve."""

import pytest

from nlab import build


@pytest.fixture
def calls(tmp_path, monkeypatch):
    log = []
    monkeypatch.setattr(build, "DIST", tmp_path)
    monkeypatch.setattr(build, "build_site", lambda argv: log.append(("build", argv)))
    monkeypatch.setattr(build, "serve", lambda d, b, p: log.append(("serve", d, b, p)))
    return log


def test_builds_first_when_no_site(calls, tmp_path):
    build.serve_site(["--downloads", "x.json"])
    assert calls == [("build", ["--downloads", "x.json"]),
                     ("serve", tmp_path / "html", "127.0.0.1", 8000)]


def test_serves_existing_site_without_building(calls, tmp_path):
    (tmp_path / "html").mkdir()
    (tmp_path / "html/search.html").write_text("ok")
    build.serve_site(["--port", "8123"])
    assert calls == [("serve", tmp_path / "html", "127.0.0.1", 8123)]


def test_rebuild_forces_a_build(calls, tmp_path):
    (tmp_path / "html").mkdir()
    (tmp_path / "html/search.html").write_text("ok")
    build.serve_site(["--rebuild", "--jobs", "4"])
    assert calls[0] == ("build", ["--jobs", "4"])


def test_build_options_without_rebuild_are_rejected(calls, tmp_path):
    (tmp_path / "html").mkdir()
    (tmp_path / "html/search.html").write_text("ok")
    with pytest.raises(SystemExit):
        build.serve_site(["--downloads", "x.json"])
    assert calls == []


def test_failed_build_serves_nothing(tmp_path, monkeypatch):
    served = []
    monkeypatch.setattr(build, "DIST", tmp_path)

    def failing_build(argv):
        raise SystemExit(1)
    monkeypatch.setattr(build, "build_site", failing_build)
    monkeypatch.setattr(build, "serve", lambda *a: served.append(a))
    with pytest.raises(SystemExit):
        build.serve_site([])
    assert served == []
