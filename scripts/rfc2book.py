#!/usr/bin/env python3
# This script is based on the `generate-book.py` script from the Rust RFCs
# repository: https://github.com/rust-lang/rfcs/blob/2b78d7bd05f718dc7e5023372f8a692d3b448600/generate-book.py

"""
This auto-generates the mdBook SUMMARY.md file based on the layout on the filesystem.

This generates the `src` directory based on the contents of the `text` directory.

Most RFCs should be kept to a single chapter. However, in some rare cases it
may be necessary to spread across multiple pages. In that case, place them in
a subdirectory with the same name as the RFC. For example:

    0123-my-awesome-feature.md
    0123-my-awesome-feature/extra-material.md

It is recommended that if you have static content like images that you use a similar layout:

    0123-my-awesome-feature.md
    0123-my-awesome-feature/diagram.svg

The chapters are presented in sorted-order.
"""

import os
import shutil
import subprocess



def main():
    src_path = None
    rfc_path = None
    cwd = os.getcwd()
    if cwd.endswith('book'):
        src_path = 'src'
        rfc_path = '../rfcs'
    elif cwd.endswith('mnemos'):
        src_path = 'book/src'
        rfc_path = 'rfcs'
    else:
        raise Exception('rfc2book must be run either in the repo root (mnemos/) or in the mdbook (mnemos/book/) directory')

    book_rfcs = f'{src_path}/rfcs'

    if os.path.exists(book_rfcs):
        # Clear out src to remove stale links in case you switch branches.
        shutil.rmtree(book_rfcs)
    os.mkdir(book_rfcs)

    with open(f'{src_path}/index.md', 'r') as summary_in:
        summary = summary_in.read()
        with open(f'{src_path}/SUMMARY.md', 'w') as summary_out:
            summary_out.write(summary)
            index_item = '\n# RFCs\n\n- [Introduction](rfcs/README.md)\n';
            print(index_item, end='')
            summary_out.write(f'\n{index_item}')
            collect(summary_out, rfc_path, src_path, 0)
            print('')

def collect(summary, path, srcpath, depth):
    entries = [e for e in os.scandir(path) if e.name.endswith('.md')]
    entries.sort(key=lambda e: e.name)
    for entry in entries:
        symlink(f'../../../{path}/{entry.name}', f'{srcpath}/rfcs/{entry.name}')
        indent = '    '*depth
        name = entry.name[:-3]
        if name != 'README':
            link_path = entry.path[5:]
            index_item = f'- {indent}[{name}](rfcs/{link_path})\n'
            print(index_item, end='')
            summary.write(index_item)
            maybe_subdir = os.path.join(path, name)
            if os.path.isdir(maybe_subdir):
                collect(summary, maybe_subdir, srcpath, depth+1)

def symlink(src, dst):
    if not os.path.exists(dst):
        os.symlink(src, dst)

if __name__ == '__main__':
    main()