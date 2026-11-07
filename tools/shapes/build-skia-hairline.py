#!/usr/bin/env python3
import argparse
import fcntl
import gzip
import hashlib
import json
import platform
import subprocess
import tarfile
import tomllib
import tempfile
from pathlib import Path

_FACTS = tomllib.loads((Path(__file__).resolve().parents[2] / "skia.toml").read_text())
SKIA_REVISION = _FACTS["skia_revision"]
ARCHIVE_SHA256 = _FACTS["skia_archive_sha256"]
VERSION = _FACTS["version"]
DEFINES = [
    "SK_CODEC_DECODES_BMP",
    "SK_CODEC_DECODES_WBMP",
    "SK_ENABLE_PRECOMPILE",
    "SK_ASSUME_GL=1",
    "SK_DISABLE_TRACING",
    "SK_USE_PERFETTO",
    "SK_ENABLE_API_AVAILABLE",
    "SK_GAMMA_APPLY_TO_A8",
    "SK_ENABLE_AVX512_OPTS",
    "SKIA_IMPLEMENTATION=1",
    "SK_TYPEFACE_FACTORY_CORETEXT",
    "SK_FONTMGR_CORETEXT_AVAILABLE",
    "SK_CODEC_DECODES_ICO",
    "SK_CODEC_DECODES_PNG",
    "SK_CODEC_DECODES_PNG_WITH_LIBPNG",
    "SK_CODEC_ENCODES_PNG",
    "SK_CODEC_ENCODES_PNG_WITH_LIBPNG",
    "SK_HAS_WUFFS_LIBRARY",
    "SK_CODEC_DECODES_GIF",
    "SK_XML",
    "SK_SUPPORT_LEGACY_CONIC_CHOP",
    "SK_USE_PADDED_BLUR_UPSCALE",
    "SK_LEGACY_INITWITHPREV_LAYER_SIZING",
    "SK_AVOID_SLOW_RASTER_PIPELINE_BLURS",
    "SK_SUPPORT_LEGACY_RRECT_TRANSFORM",
    "SK_SUPPORT_LEGACY_ANISOTROPIC_MIPMAP_SCALE",
    "SK_USE_SAFE_INSET_FOR_TEXTURE_SAMPLING",
    "SK_ENABLE_LEGACY_SHADERCONTEXT",
]


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def run(command):
    try:
        return subprocess.run(command, check=True, text=True, capture_output=True)
    except subprocess.CalledProcessError as error:
        raise SystemExit(f"{command[0]} failed:\n{error.stderr}")


def add_file(archive, path, name):
    info = tarfile.TarInfo(name)
    info.size = path.stat().st_size
    info.mode = 0o644
    info.uid = 0
    info.gid = 0
    info.uname = ""
    info.gname = ""
    info.mtime = 0
    with path.open("rb") as source:
        archive.addfile(info, source)


def parse_args():
    parser = argparse.ArgumentParser()
    for name in ("source", "target", "lock", "work-dir", "output", "receipt"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    return parser.parse_args()


def check_inputs(args):
    if platform.machine() not in {"arm64", "aarch64"}:
        raise SystemExit("wrapper package requires an Apple-silicon build host")
    if not args.target.is_dir() or not args.lock.is_file():
        raise SystemExit("the assigned shared Skia target or lock is missing")
    if run(["git", "-C", str(args.source), "rev-parse", "HEAD"]).stdout.strip() != SKIA_REVISION:
        raise SystemExit("Skia source revision does not match the approved revision")
    if run(["git", "-C", str(args.source), "status", "--porcelain"]).stdout:
        raise SystemExit("Skia source tree contains uncommitted changes")


def find_archive(target):
    archives = sorted(target.glob("debug/build/skia-bindings-*/out/skia/libskia.a"))
    archive = next((path for path in archives if sha256(path) == ARCHIVE_SHA256), None)
    if archive is None:
        raise SystemExit("the unchanged installed Skia archive was not found")
    return archive


def compile_wrapper(directory, compiler, flags, source, source_file):
    obj = directory / "scanner.o"
    library = directory / "libskia_hairline.a"
    run([compiler, *flags, "-I", str(source), "-c", str(source_file), "-o", str(obj)])
    run(["/usr/bin/env", "ZERO_AR_DATE=1", "/usr/bin/libtool", "-static", "-o", str(library), str(obj)])
    return obj, library


def write_package(output, library, manifest_path):
    with output.open("wb") as raw:
        with gzip.GzipFile(filename="", fileobj=raw, mode="wb", mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w") as package:
                add_file(package, library, "libskia_hairline.a")
                add_file(package, manifest_path, "manifest.json")


def build_package(args, before):
    compiler = "/usr/bin/clang++"
    flags = ["-std=c++20", "-O2", "-fno-rtti", "-fno-exceptions", "-DNDEBUG", *(f"-D{d}" for d in DEFINES)]
    repository = Path(__file__).resolve().parents[2]
    source_file = repository / "crates/skia-hairline-sys/native/scanner.cpp"
    with tempfile.TemporaryDirectory(dir=args.work_dir, prefix="wrapper-") as first, tempfile.TemporaryDirectory(
        dir=args.work_dir, prefix="wrapper-"
    ) as second:
        builds = [compile_wrapper(Path(d), compiler, flags, args.source, source_file) for d in (first, second)]
        object_hashes = [sha256(obj) for obj, _ in builds]
        library_hashes = [sha256(library) for _, library in builds]
        if object_hashes[0] != object_hashes[1] or library_hashes[0] != library_hashes[1]:
            raise SystemExit("wrapper build is not reproducible")
        manifest = {
            "package": "skia-hairline-sys",
            "version": VERSION,
            "target": "aarch64-apple-darwin",
            "skia_revision": SKIA_REVISION,
            "skia_archive_sha256": before,
            "library_sha256": library_hashes[0],
            "object_sha256": object_hashes[0],
            "compiler": run([compiler, "--version"]).stdout.splitlines()[0],
            "flags": [*flags, "-I", str(args.source), "-c", str(source_file)],
        }
        manifest_path = args.work_dir / "manifest.json"
        manifest_path.write_text(json.dumps(manifest, sort_keys=True, separators=(",", ":")) + "\n")
        write_package(args.output, builds[0][1], manifest_path)
    return manifest


def main():
    args = parse_args()
    check_inputs(args)
    with args.lock.open("r+") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        archive = find_archive(args.target)
        before = sha256(archive)
        for directory in (args.work_dir, args.output.parent, args.receipt.parent):
            directory.mkdir(parents=True, exist_ok=True)
        manifest = build_package(args, before)
        after = sha256(archive)
        if before != after or after != ARCHIVE_SHA256:
            raise SystemExit("the installed Skia archive changed during wrapper build")
        receipt = {
            **manifest,
            "archive_path": str(archive),
            "package_sha256": sha256(args.output),
            "archive_sha256_before": before,
            "archive_sha256_after": after,
            "reproducible_builds": 2,
        }
        args.receipt.write_text(json.dumps(receipt, indent=2) + "\n")


if __name__ == "__main__":
    main()
