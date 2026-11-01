"""nlab.downloads: rules config, URL scanning, content sniffing."""

import json

import pytest

from nlab import downloads as d
from nlab.paths import DOWNLOAD_PRESETS


def config(tmp_path, *rules, **extra):
    p = tmp_path / "c.json"
    p.write_text(json.dumps({"rules": list(rules), **extra}))
    return d.Config([p])


def test_first_match_wins_and_disable(tmp_path):
    cfg = config(tmp_path,
                 {"name": "off", "host": "doi.org", "disable": True},
                 {"name": "pdfs", "host": "*", "path": "*.pdf"})
    assert cfg.match("https://doi.org/x.pdf").name == "off"
    assert cfg.match("https://example.org/A/B.PDF").name == "pdfs"  # case-insensitive
    assert cfg.match("https://example.org/page.html") is None
    assert cfg.match("ftp://example.org/x.pdf") is None


def test_unknown_keys_are_errors(tmp_path):
    with pytest.raises(SystemExit, match="max_concurent"):
        config(tmp_path, {"name": "x", "max_concurent": 3})


def test_presets_extend_in_order():
    cfg = d.Config([DOWNLOAD_PRESETS / "everything.json"])
    assert [r.name for r in cfg.rules][0] == "ncatlab-uploads"
    assert cfg.rules[-1].name == "any-pdf"
    # common.json's disabled hosts stay disabled under the catch-all
    assert cfg.match("https://link.springer.com/content/x.pdf").disable


def test_arxiv_rewrite():
    cfg = d.Config([DOWNLOAD_PRESETS / "common.json"])
    for url in ("http://arXiv.org/abs/0704.2030", "https://www.arxiv.org/pdf/0704.2030.pdf",
                "https://arxiv.org/abs/math/0601001v2#x"):
        rule = cfg.match(url)
        assert rule.name == "arxiv"
        assert rule.fetch_url(url).startswith("https://arxiv.org/pdf/")
    assert cfg.match("https://arxiv.org/abs/") is None  # no id: not a paper


def test_urls_in_covers_all_reference_forms():
    src = """
see [this](http://example.org/a.pdf). and <img src="/nlab/files/pic.png"/>
[[Some File.pdf:file]] and [[Other.pdf:file|text]]
\\begin{imagefromfile}
    "file_name": "fig 1.jpg",
\\end{imagefromfile}
"""
    urls = d.urls_in(src)
    assert "http://example.org/a.pdf" in urls
    assert "https://ncatlab.org/nlab/files/pic.png" in urls
    assert "https://ncatlab.org/nlab/files/Some+File.pdf" in urls
    assert "https://ncatlab.org/nlab/files/Other.pdf" in urls
    assert "https://ncatlab.org/nlab/files/fig+1.jpg" in urls


@pytest.mark.parametrize("head,kind", [
    (b"%PDF-1.7\n", "pdf"),
    (b"\x89PNG\r\n\x1a\n....", "png"),
    (b"\xff\xd8\xff\xe0", "jpg"),
    (b"GIF89a", "gif"),
    (b"RIFF\0\0\0\0WEBPVP8 ", "webp"),
    (b'<?xml version="1.0"?><svg xmlns="...">', "svg"),
    (b"<!DOCTYPE html><html><body>not found", "html"),
    (b"\x00\x01garbage", None),
])
def test_sniff(head, kind):
    assert d.sniff(head) == kind
