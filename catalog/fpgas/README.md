# FPGAs

The FPGAs Logix Engine supports, one file per FPGA family (`<family>.json`), in the format in [docs/model.md](../docs/model.md). They list every FPGA the toolchains support, so boards never add FPGAs. The data is free to use, copy and change, under MIT or Apache 2.0.

> [!IMPORTANT]
> **Use it through Logix Engine.** The intended way for tools, scripts and AI agents to read Logix Catalog is [Logix Engine](https://www.openlogix.org/engine/)'s API (`get_catalog`), which downloads the catalog and serves it. The data is free to use and copy under MIT or Apache 2.0, but reading the files directly is at your own risk: their layout may change between releases.
