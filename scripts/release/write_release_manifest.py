#!/usr/bin/env python3

import argparse
import json
from pathlib import Path


def parse_args():
    parser = argparse.ArgumentParser(
        description="Write a machine-readable raiko2 release manifest."
    )
    parser.add_argument("--version", required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--git-sha", required=True)
    parser.add_argument("--image-tag", required=True)
    parser.add_argument("--image-digest-ref", required=True)
    parser.add_argument("--guest-digests", required=True)
    parser.add_argument("--output", required=True)
    return parser.parse_args()


def load_json(path: Path):
    with path.open("r", encoding="utf-8") as f:
        return json.load(f)


def main():
    args = parse_args()

    guest_digests_path = Path(args.guest_digests)
    output_path = Path(args.output)

    guest_digests = load_json(guest_digests_path)

    manifest = {
        "version": args.version,
        "tag": args.tag,
        "git_sha": args.git_sha,
        "runtime_image": {
            "tag": args.image_tag,
            "digest_ref": args.image_digest_ref,
            "guest_backends": ["risc0", "sp1"],
        },
        "guest_digests": guest_digests,
    }

    output_path.parent.mkdir(parents=True, exist_ok=True)
    with output_path.open("w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2, sort_keys=True)
        f.write("\n")

    print(f"Wrote release manifest to {output_path}")


if __name__ == "__main__":
    main()
