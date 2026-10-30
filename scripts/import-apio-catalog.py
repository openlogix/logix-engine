#!/usr/bin/env python3
"""Converts Apio's board and FPGA definitions into the engine's catalog,
catalog/boards.json and catalog/fpgas.json.

The catalog in this repo is now its own source and may have local edits.
This script rewrites both files, so use it only to bring in more boards from
Apio: then keep the new entries and restore any local edits it overwrote.

Usage: scripts/import-apio-catalog.py [APIO_PACKAGES_FOLDER]

APIO_PACKAGES_FOLDER defaults to ~/.apio/packages. It must hold Apio's
definitions package (boards.jsonc, programmers.jsonc) and oss-cad-suite
package (FPGAS.jsonc). Review the diff after running it.

What's imported (Milestone 1):
- Every iCE40 FPGA.
- The iCE40 boards whose programmer the engine supports: openfpgaloader and
  iceprog. Each gets one programming variant, "flash", with Apio's full
  command arguments. Apio's ${BIN_FILE} becomes ${BITSTREAM}, added at the
  end when Apio leaves it out.
"""

import json
import os
import re
import sys

ARCHITECTURES = {"ice40"}
PROGRAMMERS = {"openfpgaloader", "iceprog"}


def load_jsonc(path):
    """Reads JSON with // and /* */ comments and trailing commas."""
    s = open(path, encoding="utf-8").read()
    out, i, n, in_string = [], 0, len(s), False
    while i < n:
        c = s[i]
        if in_string:
            out.append(c)
            if c == "\\":
                out.append(s[i + 1])
                i += 2
                continue
            if c == '"':
                in_string = False
            i += 1
        elif c == '"':
            in_string = True
            out.append(c)
            i += 1
        elif s.startswith("//", i):
            while i < n and s[i] != "\n":
                i += 1
        elif s.startswith("/*", i):
            i = s.index("*/", i) + 2
        else:
            out.append(c)
            i += 1
    return json.loads(re.sub(r",(\s*[}\]])", r"\1", "".join(out)))


def build_info(folder):
    try:
        with open(os.path.join(folder, "BUILD-INFO.json"), encoding="utf-8") as f:
            info = json.load(f)
        return f"{info.get('package-name')} {info.get('release-tag')}"
    except OSError:
        return os.path.basename(folder)


def main():
    packages = sys.argv[1] if len(sys.argv) > 1 else os.path.expanduser("~/.apio/packages")
    definitions = os.path.join(packages, "definitions")
    oss = os.path.join(packages, "oss-cad-suite")
    apio_fpgas = load_jsonc(os.path.join(oss, "FPGAS.jsonc"))
    apio_boards = load_jsonc(os.path.join(definitions, "boards.jsonc"))
    apio_programmers = load_jsonc(os.path.join(definitions, "programmers.jsonc"))

    fpgas = {}
    for fid, f in sorted(apio_fpgas.items()):
        if f["arch"] not in ARCHITECTURES:
            continue
        params = f[f"{f['arch']}-params"]
        fpgas[fid] = {
            "architecture": f["arch"],
            "part_number": f["vendor-part"],
            "size": f["size"],
            "nextpnr_device": params["nextpnr-type"],
            "nextpnr_package": params["nextpnr-package"],
        }

    boards, skipped = {}, []
    for bid, b in sorted(apio_boards.items()):
        if b["fpga-id"] not in fpgas:
            continue
        pid = b["programmer"]["id"]
        if pid not in PROGRAMMERS or "usb" not in b:
            skipped.append(f"{bid} ({pid})")
            continue
        args = " ".join(
            a for a in [apio_programmers[pid].get("args", ""), b["programmer"].get("extra-args", "")] if a
        )
        args = args.replace("${BIN_FILE}", "${BITSTREAM}")
        if "${BITSTREAM}" not in args:
            args += " ${BITSTREAM}"
        usb = {"vid": b["usb"]["vid"].lower(), "pid": b["usb"]["pid"].lower()}
        if "product-regex" in b["usb"]:
            usb["product_regex"] = b["usb"]["product-regex"]
        boards[bid] = {
            "description": b["description"],
            "fpga_id": b["fpga-id"],
            "default_variant": "flash",
            "variants": {"flash": {"programmer": pid, "args": args.strip(), "usb": usb}},
        }

    root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "catalog")
    source = f"Imported from Apio's {build_info(definitions)} and {build_info(oss)} packages by scripts/import-apio-catalog.py."
    for name, data in [("fpgas", fpgas), ("boards", boards)]:
        with open(os.path.join(root, f"{name}.json"), "w", encoding="utf-8") as f:
            json.dump({"source": source, name: data}, f, indent=2, ensure_ascii=False)
            f.write("\n")
    print(f"{len(fpgas)} FPGAs, {len(boards)} boards. Skipped {len(skipped)} boards with unsupported programmers:")
    for s in skipped:
        print(f"  {s}")


if __name__ == "__main__":
    main()
