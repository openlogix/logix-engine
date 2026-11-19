# Boards

One file per board, `<id>.json`, in the format in [docs/model.md](../docs/model.md). The data is free to use, copy and change, under MIT or Apache 2.0.

> [!IMPORTANT]
> **Use it through Logix Engine.** The intended way for tools, scripts and AI agents to read Logix Catalog is [Logix Engine](https://www.openlogix.org/engine/)'s API (`get_catalog`), which downloads the catalog and serves it. The data is free to use and copy under MIT or Apache 2.0, but reading the files directly is at your own risk: their layout may change between releases.

Through the engine, tools get the catalog version the engine builds and programs with, custom boards read the same way, and what the engine derives from the data.

**Adding or fixing a board:** edit its file here and open a pull request; see [Adding or fixing a board](../README.md#adding-or-fixing-a-board). Your own boards use the same format, in your project's boards folder.
