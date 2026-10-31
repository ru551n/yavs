# Writing the documentation

The site is built by Sphinx from the files in this directory and published by Read the Docs.
It is a user guide: short, task-oriented pages, examples first, no internals. Only the pages
listed in a `toctree` of [`index.rst`](index.rst) are part of it; the rest of `docs/` (plan,
status, decisions, design notes, CLI contract, parallel kernel measurements, the full conformance
report) is for developers and agents and is listed in `exclude_patterns` in [`conf.py`](conf.py).
The site shows the summary of `conformance.md` (`conformance.rst`). Keep developer detail
(counters, cache formats, crate names, measurement method) out of the site pages; link the
developer document on GitHub instead. The exception is `environment.rst`, which lists every
environment variable, the development ones in a section of their own.

Some pages have a test that keeps them current. `error-codes.rst`, `cli-reference.rst` and
`lint-rules.rst` are generated: regenerate them with
`YAVS_BLESS=1 cargo test -p yavs --bin yavs cli_reference`.
`environment.rst` is written by hand: `cargo test -p yavs --test env_registry` fails on a variable
without a row and on a row nothing reads.

## Add a page

1. Write `docs/<name>.md` (MyST Markdown) or `docs/<name>.rst`.
2. Add `<name>` to a `toctree` in `index.rst`, the group that fits the task: Get started, Run and
   debug, Libraries and integration, Reference, Project.
3. Link to other site pages by file name (`[waveforms](waveforms.rst)`); link to files that are not part
   of the site (STATUS.md, design notes, source) with a full `https://github.com/ru551n/yavs/blob/main/...` URL,
   or the build fails.
4. Build with warnings as errors, as CI does:

```
pip install -r docs/requirements.txt
sphinx-build -W --keep-going -b html docs docs/_build/html
```

## Shape of a page

Start with one or two sentences on what the page is for. Use headings that say what the section
does, a local `contents` list on a long page, and end with a "See also" list of links. File names
stay flat, since the README, `llms.txt`, Context7 and external sites link to them. To retire a page or
a section, keep a stub with the old heading that points to the new place. One page does one job: the
reference pages (`cli-reference.rst`, `vunit-yavs.rst`, `environment.rst`, `error-codes.rst`) list
everything, and the guide pages link there instead of repeating them. Commands and output on a
page are run on a current build; the examples in `docs/examples` are run by
`crates/yavs/tests/docs_examples.rs` and `python/vunit-yavs/tests/test_real_yavs.py`. Say when a
feature needs a release newer than the last one.

## llms.txt

The build also writes `llms.txt` (an index) and `llms-full.txt` (all pages as one text) into
the root of the site, for language-model tools such as Context7. Nothing to do per page; every
page in a `toctree` is included.
