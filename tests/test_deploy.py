"""nlab.deploy: where the Netlify site comes from."""

import json

import pytest

from nlab import deploy


def test_netlify_site_resolution(tmp_path, monkeypatch):
    monkeypatch.setattr(deploy, "ROOT", tmp_path)
    monkeypatch.delenv("NLAB_NETLIFY_SITE", raising=False)
    with pytest.raises(SystemExit, match="netlify link --id"):
        deploy.netlify_site(None)
    # a `netlify link`ed checkout: let the CLI use its own state
    (tmp_path / ".netlify").mkdir()
    (tmp_path / ".netlify/state.json").write_text(json.dumps({"siteId": "abc"}))
    site, source = deploy.netlify_site(None)
    assert site is None and "abc" in source
    monkeypatch.setenv("NLAB_NETLIFY_SITE", "from-env")
    assert deploy.netlify_site(None)[0] == "from-env"
    assert deploy.netlify_site("explicit")[0] == "explicit"
