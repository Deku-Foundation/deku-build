#!/usr/bin/env python3
"""Verify build-result output declarations against a completed staging tree.

This verifies file identity (path, type, size and SHA-256) only. It does not
authenticate the result record, the builder, or the worker's isolation claims.
The worker must be stopped and its staging tree must no longer be writable by
untrusted processes before this verifier is run.
"""
from __future__ import annotations

import argparse
import errno
import hashlib
import json
import os
import re
import stat
import sys
from typing import Any

MAX_RESULT_BYTES = 1024 * 1024
MAX_OUTPUT_COUNT = 4096
DEFAULT_MAX_TOTAL_BYTES = 16 * 1024 * 1024 * 1024
MAX_PATH_BYTES = 200
_HEX_SHA256 = re.compile(r"^[0-9a-f]{64}$")


class VerificationError(ValueError):
    """Raised when a result or output tree violates the verification policy."""


def _unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise VerificationError(f"duplicate JSON object key: {key}")
        result[key] = value
    return result


def _safe_relative_path(value: Any) -> bool:
    if not isinstance(value, str) or not value or len(value.encode("utf-8")) > MAX_PATH_BYTES:
        return False
    if value.startswith("/") or "\\" in value:
        return False
    if not (value[0].isascii() and value[0].isalnum()):
        return False
    if not all(c.isascii() and (c.isalnum() or c in "._-/") for c in value):
        return False
    parts = value.split("/")
    return all(part not in ("", ".", "..") and part.lower() != ".git" for part in parts)


def _parse_result(path: str) -> list[dict[str, Any]]:
    if not hasattr(os, "O_NOFOLLOW") or not hasattr(os, "O_CLOEXEC"):
        raise VerificationError("this verifier requires Linux-style O_NOFOLLOW support")
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK)
    except OSError as error:
        if error.errno == errno.ELOOP:
            raise VerificationError("result path must be a regular non-symlink file") from error
        raise VerificationError(f"cannot safely open result file: {error}") from error
    try:
        metadata = os.fstat(fd)
        if not stat.S_ISREG(metadata.st_mode):
            raise VerificationError("result path must be a regular non-symlink file")
        if metadata.st_size > MAX_RESULT_BYTES:
            raise VerificationError(f"result file exceeds {MAX_RESULT_BYTES} bytes")
        with os.fdopen(os.dup(fd), "rb") as stream:
            raw = stream.read(MAX_RESULT_BYTES + 1)
    except OSError as error:
        raise VerificationError(f"cannot read result file: {error}") from error
    finally:
        os.close(fd)
    if len(raw) > MAX_RESULT_BYTES:
        raise VerificationError(f"result file exceeds {MAX_RESULT_BYTES} bytes")
    try:
        result = json.loads(raw.decode("utf-8"), object_pairs_hook=_unique_object)
    except (UnicodeDecodeError, json.JSONDecodeError, VerificationError, RecursionError) as error:
        raise VerificationError(f"invalid result JSON: {error}") from error

    if not isinstance(result, dict) or result.get("schema_version") != 1:
        raise VerificationError("result must be an object with schema_version=1")
    if result.get("outcome") != "success":
        raise VerificationError("only successful build results can be output-verified")
    outputs = result.get("outputs")
    if not isinstance(outputs, list) or not outputs or len(outputs) > MAX_OUTPUT_COUNT:
        raise VerificationError(f"outputs must contain 1 to {MAX_OUTPUT_COUNT} declarations")

    seen: dict[str, dict[str, Any]] = {}
    for index, item in enumerate(outputs):
        if not isinstance(item, dict) or set(item) != {"path", "kind", "size_bytes", "sha256"}:
            raise VerificationError(f"outputs[{index}] has an invalid shape")
        output_path = item["path"]
        if not _safe_relative_path(output_path):
            raise VerificationError(f"unsafe output path: {output_path!r}")
        if output_path in seen:
            raise VerificationError(f"duplicate output path: {output_path}")
        if item["kind"] not in {"package", "archive", "file"}:
            raise VerificationError(f"invalid output kind: {output_path}")
        size = item["size_bytes"]
        if isinstance(size, bool) or not isinstance(size, int) or size <= 0:
            raise VerificationError(f"invalid output size: {output_path}")
        digest = item["sha256"]
        if not isinstance(digest, str) or not _HEX_SHA256.fullmatch(digest):
            raise VerificationError(f"invalid output SHA-256: {output_path}")
        seen[output_path] = item

    paths = sorted(seen)
    for index, current in enumerate(paths[:-1]):
        next_path = paths[index + 1]
        if next_path.startswith(current + "/"):
            raise VerificationError(f"output path collision: {current} and {next_path}")
    return [seen[key] for key in paths]


def _open_directory(path: str) -> int:
    required_flags = ("O_DIRECTORY", "O_NOFOLLOW", "O_CLOEXEC")
    if any(not hasattr(os, name) for name in required_flags):
        raise VerificationError("this verifier requires Linux-style O_DIRECTORY/O_NOFOLLOW support")
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
    try:
        fd = os.open(path, flags)
    except OSError as error:
        raise VerificationError(f"output root must be a real non-symlink directory: {error}") from error
    metadata = os.fstat(fd)
    if not stat.S_ISDIR(metadata.st_mode):
        os.close(fd)
        raise VerificationError("output root is not a directory")
    if metadata.st_mode & 0o022:
        os.close(fd)
        raise VerificationError("output root must not be group- or world-writable")
    return fd


def _open_regular_relative(root_fd: int, relative_path: str) -> tuple[int, os.stat_result]:
    parts = relative_path.split("/")
    parent_fd = os.dup(root_fd)
    try:
        for part in parts[:-1]:
            try:
                next_fd = os.open(
                    part,
                    os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                    dir_fd=parent_fd,
                )
            except OSError as error:
                raise VerificationError(f"unsafe or missing parent directory for {relative_path}: {error}") from error
            os.close(parent_fd)
            parent_fd = next_fd
        try:
            file_fd = os.open(
                parts[-1],
                os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK,
                dir_fd=parent_fd,
            )
        except OSError as error:
            raise VerificationError(f"cannot safely open output {relative_path}: {error}") from error
    finally:
        os.close(parent_fd)

    metadata = os.fstat(file_fd)
    if not stat.S_ISREG(metadata.st_mode):
        os.close(file_fd)
        raise VerificationError(f"output is not a regular file: {relative_path}")
    if metadata.st_mode & (stat.S_ISUID | stat.S_ISGID):
        os.close(file_fd)
        raise VerificationError(f"setuid/setgid output is forbidden: {relative_path}")
    if metadata.st_nlink != 1:
        os.close(file_fd)
        raise VerificationError(f"hard-linked output is forbidden: {relative_path}")
    if metadata.st_mode & 0o022:
        os.close(file_fd)
        raise VerificationError(f"group/world-writable output file is forbidden: {relative_path}")
    return file_fd, metadata


def _inventory_tree(root_fd: int, declared_paths: set[str], max_entries: int) -> set[str]:
    found: set[str] = set()
    entries_seen = 0

    def walk(directory_fd: int, prefix: str) -> None:
        nonlocal entries_seen
        try:
            names = os.listdir(directory_fd)
        except OSError as error:
            raise VerificationError(f"cannot enumerate output tree: {error}") from error
        for name in names:
            entries_seen += 1
            if entries_seen > max_entries:
                raise VerificationError("output tree contains too many entries")
            relative = f"{prefix}/{name}" if prefix else name
            try:
                item = os.stat(name, dir_fd=directory_fd, follow_symlinks=False)
            except OSError as error:
                raise VerificationError(f"cannot inspect output entry {relative!r}: {error}") from error

            if stat.S_ISDIR(item.st_mode):
                if item.st_mode & 0o022:
                    raise VerificationError(f"group/world-writable output directory is forbidden: {relative}")
                if not any(path.startswith(relative + "/") for path in declared_paths):
                    raise VerificationError(f"undeclared output directory: {relative}")
                try:
                    child_fd = os.open(
                        name,
                        os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                        dir_fd=directory_fd,
                    )
                except OSError as error:
                    raise VerificationError(f"unsafe output directory {relative!r}: {error}") from error
                try:
                    walk(child_fd, relative)
                finally:
                    os.close(child_fd)
            elif stat.S_ISREG(item.st_mode):
                if relative not in declared_paths:
                    raise VerificationError(f"undeclared output file: {relative}")
                if item.st_nlink != 1:
                    raise VerificationError(f"hard-linked output is forbidden: {relative}")
                if item.st_mode & (stat.S_ISUID | stat.S_ISGID):
                    raise VerificationError(f"setuid/setgid output is forbidden: {relative}")
                found.add(relative)
            else:
                raise VerificationError(f"symlink or special file is forbidden in output tree: {relative}")

    walk(root_fd, "")
    return found


def verify_outputs(
    result_path: str,
    output_root: str,
    max_total_bytes: int = DEFAULT_MAX_TOTAL_BYTES,
) -> tuple[int, int]:
    if isinstance(max_total_bytes, bool) or not isinstance(max_total_bytes, int) or max_total_bytes <= 0:
        raise VerificationError("max_total_bytes must be a positive integer")

    outputs = _parse_result(result_path)
    declared = {item["path"] for item in outputs}
    total_declared = sum(item["size_bytes"] for item in outputs)
    if total_declared > max_total_bytes:
        raise VerificationError(f"declared output bytes exceed configured limit of {max_total_bytes}")

    root_fd = _open_directory(output_root)
    try:
        found = _inventory_tree(root_fd, declared, max_entries=MAX_OUTPUT_COUNT * 2 + len(declared))
        if found != declared:
            missing = sorted(declared - found)
            extra = sorted(found - declared)
            raise VerificationError(f"output inventory mismatch (missing={missing}, extra={extra})")

        total = 0
        for output in outputs:
            file_fd, before = _open_regular_relative(root_fd, output["path"])
            try:
                if before.st_size != output["size_bytes"]:
                    raise VerificationError(
                        f"output size mismatch for {output['path']}: expected {output['size_bytes']}, got {before.st_size}"
                    )
                digest = hashlib.sha256()
                read_total = 0
                while True:
                    chunk = os.read(file_fd, 1024 * 1024)
                    if not chunk:
                        break
                    read_total += len(chunk)
                    if read_total > output["size_bytes"] or read_total > max_total_bytes:
                        raise VerificationError(f"output grew while being read: {output['path']}")
                    digest.update(chunk)
                after = os.fstat(file_fd)
                stable_before = (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns)
                stable_after = (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns)
                if stable_before != stable_after:
                    raise VerificationError(f"output changed while being verified: {output['path']}")
                if read_total != output["size_bytes"]:
                    raise VerificationError(f"output size mismatch for {output['path']}")
                actual_digest = digest.hexdigest()
                if actual_digest != output["sha256"]:
                    raise VerificationError(
                        f"output digest mismatch for {output['path']}: expected {output['sha256']}, got {actual_digest}"
                    )
                total += read_total
            finally:
                os.close(file_fd)
        return len(outputs), total
    finally:
        os.close(root_fd)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--result", required=True, help="unsigned build-result JSON previously schema-validated by deku-build")
    parser.add_argument("--output-dir", required=True, help="completed output staging directory from a stopped worker")
    parser.add_argument("--max-total-bytes", type=int, default=DEFAULT_MAX_TOTAL_BYTES)
    args = parser.parse_args(argv)
    try:
        count, total = verify_outputs(args.result, args.output_dir, args.max_total_bytes)
    except VerificationError as error:
        print(f"verify-build-outputs: {error}", file=sys.stderr)
        return 2
    print(
        f"VERIFIED file_count={count} total_bytes={total}; "
        "checks cover declared paths, file types, sizes and SHA-256 only, not builder identity or isolation claims"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
