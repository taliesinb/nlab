"""nlab.conv: the Rust converter as seen from Python."""

import concurrent.futures

import pytest

from nlab import conv


def test_canonical_file_url():
    f = conv.canonical_file_url
    assert f("/nlab/files/x.png") == "https://ncatlab.org/nlab/files/x.png"
    assert f("http://ncatlab.org/nlab/files/x.png?a=1#b") == "https://ncatlab.org/nlab/files/x.png"
    assert f(" https://ncatlab.org/nlab/files/A+B.pdf ") == "https://ncatlab.org/nlab/files/A+B.pdf"
    assert f("https://arxiv.org/abs/1234.5678") == "https://arxiv.org/abs/1234.5678"


def test_page_links_resolve_through_page_index(tmp_path):
    idx = tmp_path / "page-index.tsv"
    idx.write_text("Yoneda lemma\t499\nYoneda\t499\n")
    c = conv.Converter(page_index=idx)
    assert c.resolve("yoneda LEMMA") == "499"
    assert c.resolve("nope") is None
    typ = c.page("See the [[Yoneda lemma]] and [[nowhere]].", "t")
    assert '#nlab-page("499")' in typ
    assert "https://ncatlab.org/nlab/show/nowhere" in typ


def test_images_embed_only_when_downloaded(tmp_path):
    fmap = tmp_path / "files.tsv"
    fmap.write_text("https://ncatlab.org/nlab/files/a.png\tfiles/abc.png\tpng\n"
                    "https://ncatlab.org/nlab/files/p.pdf\tfiles/def.pdf\tpdf\n")
    c = conv.Converter(files_map=fmap)
    typ = c.page('<img src="/nlab/files/a.png"/> <img src="/nlab/files/b.png"/>', "t")
    assert 'nlab-image("files/abc.png")' in typ
    assert '#link("/nlab/files/b.png")' in typ  # not downloaded: a link


def test_middle_is_bound():
    typ = conv.Converter().page(r"$$\left\{ x \;\middle|\; y \right\}$$", "t")
    assert "middle(" in typ and "#let middle" in typ


def test_formula_and_grid():
    cls, status, code = conv.formula(r"A \to B")
    assert status == "ok" and code
    assert conv.grid(r"\array{A & \to & B}").startswith("[")
    with pytest.raises(ValueError):
        conv.grid("x + y")


def test_tikzcd():
    code, ignored = conv.Converter().tikzcd(
        r'\begin{tikzcd} A \arrow[r, "f"] & B \end{tikzcd}')
    assert "diagram(" in code and isinstance(ignored, list)
    with pytest.raises(ValueError):
        conv.Converter().tikzcd(r'A \arrow[r] & B')  # no environment


def test_threads_give_identical_output():
    """Conversions release the GIL and must not share mutable state."""
    src = r"""
\begin{tikzcd}
A \arrow[rrr, "long label here"] & & & B \\
C \arrow[rrr, "another long label"] & & & D
\end{tikzcd}
"""
    c = conv.Converter()
    with concurrent.futures.ThreadPoolExecutor(8) as ex:
        outs = set(ex.map(lambda _: c.page(src, "t"), range(32)))
    assert len(outs) == 1
