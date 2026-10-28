# Writing the documentation

The site is built by Sphinx from the files in this directory and published by Read the Docs.
It is a user guide: short, task-oriented pages, examples first, no internals. Only the pages
listed in a `toctree` of [`index.rst`](index.rst) are part of it; the rest of `docs/` (plan,
status, decisions, design notes, CLI contract, parallel kernel measurements, the full conformance
report) is for developers and agents and is listed in `exclude_patterns` in [`conf.py`](conf.py).
The site shows the summary of `conformance.md` (`conformance.rst`). Keep developer detail
(environment variables for development, counters, cache formats, crate names, measurement
method) out of the site pages; link the developer document on GitHub instead.

## Add a page

1. Write `docs/<name>.md` (MyST Markdown) or `docs/<name>.rst`.
2. Add `<name>` to a `toctree` in `index.rst` (User guide for how-to pages, Reference for sheets).
3. Link to other site pages by file name (`[waveforms](waveforms.rst)`); link to files that are not part
   of the site (STATUS.md, design notes, source) with a full `https://github.com/ru551n/yavs/blob/main/...` URL,
   or the build fails.
4. Build with warnings as errors, as CI does:

```
pip install -r docs/requirements.txt
sphinx-build -W --keep-going -b html docs docs/_build/html
```

## llms.txt

The build also writes `llms.txt` (an index) and `llms-full.txt` (all pages as one text) into
the root of the site, for language-model tools such as Context7. Nothing to do per page; every
page in a `toctree` is included.
