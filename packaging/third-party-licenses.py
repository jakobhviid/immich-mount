#!/usr/bin/env python3
"""Write the license notices a binary package must carry: our own THIRD-PARTY-NOTICES.md (code adapted from other
projects), then every crate the given binaries link, with its license files from the cargo registry.

    packaging/third-party-licenses.py OUTPUT PACKAGE [PACKAGE ...] [--target TRIPLE]

PACKAGE is a workspace package (immich-mount, immich-mount-gnome, immich-mount-kde). The crates are those
`cargo tree -e normal` lists for the target (default: the host), so build-only and test-only crates are left out.
Run from the workspace root, after a build has fetched the crates. Standard library only.
"""

import hashlib
import json
import os
import subprocess
import sys

LICENSE_FILE_PREFIXES = ("LICENSE", "LICENCE", "COPYING", "NOTICE", "UNLICENSE")


def linked_crates(packages, target):
    """(name, version) of every crate the packages link for the target, with its license expression."""
    crates = {}
    for package in packages:
        command = ["cargo", "tree", "-p", package, "-e", "normal", "--prefix", "none", "--format", "{p}|{l}"]
        if target:
            command += ["--target", target]
        for line in subprocess.run(command, check=True, capture_output=True, text=True).stdout.splitlines():
            # "name v1.2.3|LICENSE", with " (path)" after a path dependency's version, and " (*)" at the end of the
            # line for a crate listed before.
            described, _, license_expression = line.removesuffix(" (*)").partition("|")
            parts = described.split()
            if len(parts) >= 2:
                crates[(parts[0], parts[1].lstrip("v"))] = license_expression.strip()
    return crates


def crate_directories():
    """(name, version) of every crate in the lock file to its source directory, and the workspace's own names."""
    metadata = json.loads(subprocess.run(["cargo", "metadata", "--format-version", "1"], check=True, capture_output=True, text=True).stdout)
    directories = {(package["name"], package["version"]): os.path.dirname(package["manifest_path"]) for package in metadata["packages"]}
    members = {package_id for package_id in metadata["workspace_members"]}
    ours = {(package["name"], package["version"]) for package in metadata["packages"] if package["id"] in members}
    return directories, ours


def license_files(directory):
    names = sorted(name for name in os.listdir(directory) if name.upper().startswith(LICENSE_FILE_PREFIXES))
    return [name for name in names if os.path.isfile(os.path.join(directory, name))]


def main(arguments):
    target = None
    if "--target" in arguments:
        index = arguments.index("--target")
        target = arguments[index + 1]
        del arguments[index : index + 2]
    if len(arguments) < 2:
        sys.exit(__doc__)
    output, packages = arguments[0], arguments[1:]
    crates = linked_crates(packages, target)
    directories, ours = crate_directories()
    sections = []
    with open("THIRD-PARTY-NOTICES.md", encoding="utf-8") as notices:
        sections.append(notices.read().rstrip() + "\n")
    sections.append(
        f"# Crates linked into {', '.join(packages)}\n\n"
        "Each crate with the license it is used under and the license files it ships, as fetched by cargo.\n"
    )
    # The same license text (Apache-2.0, MIT) comes with most crates: it is printed once, and later crates point to it.
    printed = {}
    for name, version in sorted(crates):
        if (name, version) in ours:
            continue
        expression = crates[(name, version)] or "no license expression"
        section = [f"\n## {name} {version} ({expression})\n"]
        directory = directories.get((name, version))
        files = license_files(directory) if directory else []
        for file_name in files:
            with open(os.path.join(directory, file_name), encoding="utf-8", errors="replace") as text:
                body = text.read().rstrip()
            digest = hashlib.sha256(body.encode()).hexdigest()
            if digest in printed:
                section.append(f"\n### {file_name}\n\nThe same text as {printed[digest]}.\n")
            else:
                printed[digest] = f"{file_name} of {name} {version}"
                section.append(f"\n### {file_name}\n\n```text\n{body}\n```\n")
        if not files:
            section.append(f"\nThe crate ships no license file; it is used under {expression}.\n")
        sections.append("".join(section))
    with open(output, "w", encoding="utf-8") as out:
        out.write("\n".join(sections))
    print(f"{output}: {sum(1 for crate in crates if crate not in ours)} crates")


if __name__ == "__main__":
    main(sys.argv[1:])
