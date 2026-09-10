#!/usr/bin/env python3
"""Convert an extracted Fallout 4 worldspace package into a native North Star YMAP.

Authoritative source:
  <package>/metadata/visual_instances.json
  <package>/Data/Meshes/**

Pipeline:
  Fallout4.esm REFR transforms + reusable NIFs
      -> PyNifly/Blender headless NIF geometry bake
      -> meter-scale North Star OBJ sources
      -> reusable YDD + YTYP definitions
      -> YMAP v2 cells preserving every placement transform

Fallout-specific parsing/conversion remains an import-time tool concern. Runtime
only sees native YDD/YTYP/YMAP assets.
"""
from __future__ import annotations

import argparse
import json
import math
import os
import shutil
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET
from pathlib import Path
from typing import Any

_SCRIPT_DIR = Path(__file__).resolve().parent
if str(_SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(_SCRIPT_DIR))

import import_blender_map as native


IMPORT_SCHEMA = "northstar.fallout4_map_import.v1"
BATCH_SCHEMA = "northstar.fallout4_nif_batch_export.v1"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "package",
        type=Path,
        help="Extracted Fallout 4 map package containing Data/ and metadata/visual_instances.json",
    )
    parser.add_argument("--map-id", default="sanctuary_hills_prewar")
    parser.add_argument("--output", help="Runtime logical .ymap path; defaults to maps/<map-id>.ymap")
    parser.add_argument("--root", type=Path, help="NorthStar repository root")
    parser.add_argument("--project", default="SanctuaryHillsPreWar", help="Project owner directory under Projects/")
    parser.add_argument("--batch-manifest", type=Path, help="Reuse a completed Fallout NIF batch result instead of rerunning Blender")
    parser.add_argument("--blender", type=Path, help="Blender executable override")
    parser.add_argument("--pynifly-root", type=Path, help="PyNifly checkout containing io_scene_nifly")
    parser.add_argument(
        "--units-per-meter",
        type=float,
        default=70.0,
        help="Bethesda world units per North Star meter (default 70)",
    )
    parser.add_argument(
        "--cell-size",
        type=float,
        help="Native cell size in meters; defaults to one Bethesda exterior cell (4096/units-per-meter)",
    )
    parser.add_argument(
        "--no-build",
        action="store_true",
        help="Generate/update authoring sources without compiling YDD/YTYP/YMAP runtime assets",
    )
    parser.add_argument("--dry-run", action="store_true", help="Inspect source inventory without repository writes")
    return parser.parse_args()



def repository_root(raw: Path | None) -> Path:
    if raw is not None:
        root = raw.expanduser().resolve()
        if (root / "NewEngine" / "neocore2").is_dir() and (root / "Shared" / "Content").is_dir() and (root / "Projects").is_dir():
            return root
        raise SystemExit(f"invalid NorthStar root: {root}")
    here = Path(__file__).resolve()
    for candidate in (Path.cwd().resolve(), *Path.cwd().resolve().parents, *here.parents):
        if (candidate / "NewEngine" / "neocore2").is_dir() and (candidate / "Shared" / "Content").is_dir() and (candidate / "Projects").is_dir():
            return candidate
    raise SystemExit("NorthStar repository root not found; pass --root")


def owner_build_plan(
    *,
    project_name: str,
    map_id: str,
    model_records: list[dict[str, Any]],
    definition_records: list[dict[str, Any]],
    map_record: dict[str, Any],
) -> dict[str, Any]:
    return {
        "schema": "northstar.native_asset_build_plan.v1",
        "version": 2,
        "id": native.slug(project_name),
        "asset_root": f"Projects/{project_name}",
        "policy": [
            "Fallout 4 source content is import-time provenance only; runtime consumes native NorthStar assets.",
            "Sanctuary Hills Pre-War base map preserves all 4017 extracted REFR placement transforms.",
            "Creation Kit editor markers retain placement identity but are metadata-only and disabled at runtime.",
        ],
        "blocked_sources": [],
        "textures": [],
        "models": model_records,
        "hair": [],
        "fonts": [],
        "ui": [],
        "materials": [],
        "definitions": definition_records,
        "spatial_pages": [],
        "definition_catalogs": [
            {
                "source_dir": f"Source/definitions/maps/{map_id}",
                "logical_prefix": f"definitions/maps/{map_id}",
                "map_ref": f"maps/{map_id}.ymap",
                "output": f"Content/maps/{map_id}.definition_catalog",
                "logical_path": f"maps/{map_id}.definition_catalog",
            }
        ],
        "maps": [map_record],
        "items": [],
        "animations": [],
        "metadata": [],
        "audio": [],
        "sound_cues": [],
        "scripts": [],
        "shaders": [],
        "validate_only": [],
        "catalog_roots": ["Content", "Source"],
        "runtime_mounts": [
            {"root": "Content", "mount": "/"},
            {"root": "Content/definitions", "mount": "definitions"},
        ],
    }


def load_completed_batch(path: Path, units_per_meter: float) -> dict[str, Any]:
    if not path.is_file():
        raise FileNotFoundError(path)
    payload = json.loads(path.read_text(encoding="utf-8"))
    if payload.get("schema") != BATCH_SCHEMA:
        raise RuntimeError(f"unexpected batch export schema: {payload.get('schema')!r}")
    actual_units = float(payload.get("units_per_meter", math.nan))
    if not math.isfinite(actual_units) or abs(actual_units - units_per_meter) > 1.0e-9:
        raise RuntimeError(
            f"batch unit scale mismatch expected={units_per_meter} actual={actual_units}"
        )
    validate_conversion(payload)
    assets = payload.get("assets") or []
    counts = payload.get("counts") or {}
    if len(assets) != 448 or int(counts.get("converted_assets", -1)) != 448:
        raise RuntimeError(f"Sanctuary asset invariant failed expected=448 actual={len(assets)}")
    render_assets = [asset for asset in assets if not bool(asset.get("editor_only"))]
    marker_assets = [asset for asset in assets if bool(asset.get("editor_only"))]
    if len(render_assets) != 443 or len(marker_assets) != 5:
        raise RuntimeError(
            f"Sanctuary asset split mismatch render={len(render_assets)} markers={len(marker_assets)}"
        )
    for asset in render_assets:
        geometry = Path(str(asset.get("geometry_file") or ""))
        if not geometry.is_file():
            raise RuntimeError(f"batch geometry missing asset={asset.get('asset_id')} path={geometry}")
    return payload


def replace_generated_tree(staged: Path, destination: Path) -> None:
    replacement = destination.with_name(destination.name + ".new")
    if replacement.exists():
        shutil.rmtree(replacement)
    replacement.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(staged, replacement)
    if destination.exists():
        shutil.rmtree(destination)
    replacement.replace(destination)


def run_owner_pipeline(repo: Path, plan_path: Path) -> None:
    tool = repo / "tools" / "maintenance" / "northstar_native_assets.py"
    command = [
        sys.executable, str(tool), "--root", str(repo), "--plan", str(plan_path),
        "build", "--only", "models", "--only", "definitions", "--only", "maps",
    ]
    print("[CMD]", subprocess.list2cmdline(command))
    built = subprocess.run(command, cwd=repo, text=True)
    if built.returncode:
        raise RuntimeError(f"NorthStar native asset build failed code={built.returncode}")
    validate = [
        sys.executable, str(tool), "--root", str(repo), "--plan", str(plan_path), "validate"
    ]
    print("[CMD]", subprocess.list2cmdline(validate))
    checked = subprocess.run(validate, cwd=repo, text=True)
    if checked.returncode:
        raise RuntimeError(f"NorthStar native asset validation failed code={checked.returncode}")

def write_marker_definition_source(path: Path, *, definition_name: str, source_model: str) -> None:
    """Write a metadata-only YTYP for a disabled Creation Kit editor marker."""
    root = ET.Element(
        "YtypProperties",
        {
            "schema": "newengine.ytyp.properties.v1",
            "representation": "xml",
            "body_format": "newengine.xml.properties.v1",
            "name": definition_name,
            "kind": "game_ready_metadata",
            "entry_kind": "archetype_definition",
            "stable_hash": str(native.fnv1a64(definition_name)),
            "flags": "0",
        },
    )
    semantic = ET.SubElement(root, "SemanticTags")
    for tag in ("map", "fallout4", "editor_marker", "metadata_only"):
        ET.SubElement(semantic, "Tag", {"value": tag})
    domain = ET.SubElement(root, "DomainTags")
    for tag in ("engine.assets.maps", "engine.assets.definitions", "engine.scene"):
        ET.SubElement(domain, "Tag", {"value": tag})
    metadata = ET.SubElement(root, "Metadata")
    source = ET.SubElement(metadata, "Namespace", {"name": "source"})
    ET.SubElement(source, "Value", {"key": "game", "value": "Fallout4"})
    ET.SubElement(source, "Value", {"key": "model", "value": source_model})
    ET.SubElement(source, "Value", {"key": "runtime.policy", "value": "disabled_editor_marker"})
    ET.indent(root, space="  ")
    path.parent.mkdir(parents=True, exist_ok=True)
    ET.ElementTree(root).write(path, encoding="utf-8", xml_declaration=True)


def normalize_rel(value: str) -> str:
    return value.replace("\\", "/").lstrip("/")


def run_batch_export(
    *,
    blender: Path,
    helper: Path,
    source_manifest: Path,
    data_root: Path,
    geometry_dir: Path,
    result_manifest: Path,
    pynifly_root: Path,
    units_per_meter: float,
) -> dict[str, Any]:
    command = [
        str(blender),
        "--background",
        "--factory-startup",
        "--python",
        str(helper),
        "--",
        "--source-manifest",
        str(source_manifest),
        "--data-root",
        str(data_root),
        "--geometry-dir",
        str(geometry_dir),
        "--result-manifest",
        str(result_manifest),
        "--pynifly-root",
        str(pynifly_root),
        "--units-per-meter",
        f"{units_per_meter:.12g}",
    ]
    print("[CMD]", subprocess.list2cmdline(command))
    result = subprocess.run(command, text=True, capture_output=True, timeout=7200)
    if result.stdout.strip():
        print(result.stdout.rstrip())
    if result.stderr.strip():
        print(result.stderr.rstrip(), file=sys.stderr)
    if not result_manifest.is_file():
        raise RuntimeError(
            f"Fallout NIF batch exporter produced no manifest; blender exit code={result.returncode}"
        )
    payload = json.loads(result_manifest.read_text(encoding="utf-8"))
    if payload.get("schema") != BATCH_SCHEMA:
        raise RuntimeError(f"unexpected batch export schema: {payload.get('schema')!r}")
    failures = payload.get("failures") or []
    if result.returncode or failures:
        first = failures[0] if failures else {}
        raise RuntimeError(
            "Fallout NIF batch export failed "
            f"code={result.returncode} failures={len(failures)} first={first.get('model_path')!r}: "
            f"{first.get('error', '')}"
        )
    return payload


def validate_conversion(payload: dict[str, Any]) -> None:
    counts = payload.get("counts") or {}
    source_instances = int(counts.get("source_instances", -1))
    converted_instances = int(counts.get("converted_instances", -1))
    missing = int(counts.get("instances_without_converted_model", -1))
    multi = int(counts.get("multi_model_instances", -1))
    if source_instances != 4017:
        raise RuntimeError(
            f"Sanctuary source invariant failed: expected 4017 placements, got {source_instances}"
        )
    if converted_instances != source_instances:
        raise RuntimeError(
            f"placement loss detected: source={source_instances} converted={converted_instances}"
        )
    if missing:
        raise RuntimeError(f"{missing} placements do not resolve to converted geometry")
    if multi:
        raise RuntimeError(
            f"{multi} placement records contain multiple model paths; "
            "refuse to guess render-model semantics"
        )
    error = float((payload.get("transform_audit") or {}).get("max_reconstructed_engine_matrix_error", math.inf))
    if not math.isfinite(error) or error > 1.0e-5:
        raise RuntimeError(f"placement transform reconstruction error too large: {error}")


def main() -> int:
    options = parse_args()
    repo = repository_root(options.root)
    package = options.package.expanduser().resolve()
    source_manifest = package / "metadata" / "visual_instances.json"
    data_root = package / "Data"
    if not source_manifest.is_file():
        raise SystemExit(f"missing visual placement manifest: {source_manifest}")
    if not data_root.is_dir():
        raise SystemExit(f"missing extracted Data directory: {data_root}")
    if not math.isfinite(options.units_per_meter) or options.units_per_meter <= 0.0:
        raise SystemExit("--units-per-meter must be finite and > 0")

    map_id = native.slug(options.map_id)
    output = normalize_rel(options.output or f"maps/{map_id}.ymap")
    if output != f"maps/{map_id}.ymap":
        raise SystemExit(
            "project-owned Fallout importer currently requires canonical output "
            f"maps/{map_id}.ymap, got {output!r}"
        )
    cell_size = (
        float(options.cell_size)
        if options.cell_size is not None
        else 4096.0 / float(options.units_per_meter)
    )
    if not math.isfinite(cell_size) or cell_size <= 0.0:
        raise SystemExit("--cell-size must be finite and > 0")

    project_name = str(options.project).strip()
    if not project_name or any(ch in project_name for ch in "\\/:"):
        raise SystemExit(f"invalid project owner name: {project_name!r}")
    project_root = repo / "Projects" / project_name
    source_doc = json.loads(source_manifest.read_text(encoding="utf-8"))
    source_instances = source_doc.get("instances") or []

    batch_path = (
        options.batch_manifest.expanduser().resolve()
        if options.batch_manifest
        else (package / "_northstar_full" / "result.json").resolve()
    )
    converted: dict[str, Any] | None = None
    if batch_path.is_file():
        converted = load_completed_batch(batch_path, options.units_per_meter)

    inventory = {
        "map_id": map_id,
        "project": project_name,
        "project_root": str(project_root),
        "source_worldspace": source_doc.get("source_worldspace"),
        "source_worldspace_form_id": source_doc.get("source_worldspace_form_id"),
        "placements": len(source_instances),
        "units_per_meter": options.units_per_meter,
        "cell_size": cell_size,
        "output": output,
        "batch_manifest": str(batch_path),
        "batch_reused": converted is not None,
    }
    if converted is not None:
        inventory["batch_counts"] = converted.get("counts")
        inventory["transform_audit"] = converted.get("transform_audit")
    if options.dry_run:
        print(json.dumps(inventory, indent=2, ensure_ascii=False))
        return 0

    if converted is None:
        helper = repo / "NewEngine" / "neocore2" / "scripts" / "fallout4_nif_batch_export.py"
        blender = native.find_blender(options.blender)
        if options.pynifly_root:
            pynifly_root = options.pynifly_root.expanduser().resolve()
        else:
            pynifly_root = (repo / "Temp" / "Tools" / "PyNiflyReleaseV28.2").resolve()
        dll = pynifly_root / "io_scene_nifly" / "NiflyDLL.dll"
        if not dll.is_file():
            raise SystemExit(f"PyNifly V28.2 native bridge is missing: {dll}")
        batch_path.parent.mkdir(parents=True, exist_ok=True)
        geometry_dir = batch_path.parent / "geometry"
        converted = run_batch_export(
            blender=blender,
            helper=helper,
            source_manifest=source_manifest,
            data_root=data_root,
            geometry_dir=geometry_dir,
            result_manifest=batch_path,
            pynifly_root=pynifly_root,
            units_per_meter=options.units_per_meter,
        )
        validate_conversion(converted)

    assert converted is not None
    validate_conversion(converted)
    assets = converted["assets"]
    placement_instances = converted["instances"]

    with tempfile.TemporaryDirectory(prefix=f"northstar-fo4-project-{map_id}-") as temporary:
        stage = Path(temporary)
        staged_project = stage / project_name
        model_source_root = staged_project / "Source" / "models" / "maps" / map_id
        definition_source_root = staged_project / "Source" / "definitions" / "maps" / map_id
        map_source = staged_project / "Source" / "maps" / f"{map_id}.ymap.xml"
        research_root = staged_project / "Source" / "Research" / "SanctuaryHillsPreWar"
        model_source_root.mkdir(parents=True, exist_ok=True)
        definition_source_root.mkdir(parents=True, exist_ok=True)
        map_source.parent.mkdir(parents=True, exist_ok=True)
        research_root.mkdir(parents=True, exist_ok=True)

        generated_definition_refs: dict[str, str] = {}
        model_records: list[dict[str, Any]] = []
        definition_records: list[dict[str, Any]] = []
        marker_count = 0
        render_count = 0

        for asset in sorted(assets, key=lambda item: str(item["asset_id"])):
            asset_id = str(asset["asset_id"])
            editor_only = bool(asset.get("editor_only"))
            definition_logical = f"definitions/maps/{map_id}/{asset_id}.ytyp"
            generated_definition_refs[asset_id] = f"{definition_logical}@{asset_id}"
            definition_records.append({
                "source": f"Source/definitions/maps/{map_id}/{asset_id}.ytyp.xml",
                "output": f"Content/definitions/maps/{map_id}/{asset_id}.ytyp",
                "logical_path": definition_logical,
            })
            definition_source = definition_source_root / f"{asset_id}.ytyp.xml"

            if editor_only:
                marker_count += 1
                write_marker_definition_source(
                    definition_source,
                    definition_name=asset_id,
                    source_model=str(asset.get("model_path") or ""),
                )
                continue

            geometry_source = Path(str(asset.get("geometry_file") or ""))
            if not geometry_source.is_file():
                raise RuntimeError(f"missing converted geometry asset={asset_id} path={geometry_source}")
            render_count += 1
            staged_obj = model_source_root / f"{asset_id}.obj"
            shutil.copy2(geometry_source, staged_obj)
            model_logical = f"models/maps/{map_id}/{asset_id}.ydd"
            model_records.append({
                "source": f"Source/models/maps/{map_id}/{asset_id}.obj",
                "output": f"Content/models/maps/{map_id}/{asset_id}.ydd",
                "logical_path": model_logical,
                "entry": asset_id,
                "properties_ref": definition_logical,
            })
            native.write_definition_source(
                definition_source,
                definition_name=asset_id,
                drawable_ref=f"{model_logical}@{asset_id}",
                material_ref="",
                collision=False,
            )

        if render_count != 443 or marker_count != 5:
            raise RuntimeError(
                f"generated asset split invariant failed render={render_count} markers={marker_count}"
            )
        if len(generated_definition_refs) != 448:
            raise RuntimeError(
                f"definition identity invariant failed expected=448 actual={len(generated_definition_refs)}"
            )

        cell_count, placement_count = native.write_map_source(
            map_source,
            map_id=map_id,
            cell_size=cell_size,
            origin=[0.0, 0.0, 0.0],
            instances=placement_instances,
            generated_definition_refs=generated_definition_refs,
        )
        if placement_count != 4017:
            raise RuntimeError(f"YMAP placement invariant failed expected=4017 actual={placement_count}")

        map_record = {
            "source": f"Source/maps/{map_id}.ymap.xml",
            "output": f"Content/maps/{map_id}.ymap",
            "logical_path": f"maps/{map_id}.ymap",
        }
        plan = owner_build_plan(
            project_name=project_name,
            map_id=map_id,
            model_records=model_records,
            definition_records=definition_records,
            map_record=map_record,
        )
        staged_plan = staged_project / "asset.build.json"
        staged_plan.write_text(
            json.dumps(plan, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )

        disabled_markers = sum(1 for item in placement_instances if not item.get("enabled", True))
        receipt = {
            "schema": IMPORT_SCHEMA,
            "map_id": map_id,
            "project": project_name,
            "source_package": str(package),
            "source_manifest": str(source_manifest),
            "source_manifest_sha256": native.sha256(source_manifest),
            "source_worldspace": source_doc.get("source_worldspace"),
            "source_worldspace_form_id": source_doc.get("source_worldspace_form_id"),
            "source_location": source_doc.get("source_location"),
            "batch_manifest": str(batch_path),
            "units_per_meter": options.units_per_meter,
            "coordinate_mapping": "FO4(x,y,z) -> NorthStar(x,z,-y) / units_per_meter",
            "cell_size": cell_size,
            "placements": placement_count,
            "cells": cell_count,
            "unique_source_models": len(assets),
            "runtime_mesh_assets": render_count,
            "metadata_marker_definitions": marker_count,
            "disabled_editor_markers": disabled_markers,
            "transform_audit": converted.get("transform_audit"),
            "output": f"maps/{map_id}.ymap",
            "preservation_policy": (
                "All 4017 source REFR transforms are retained as YMAP Placement rows; "
                "the 123 Creation Kit editor-marker placements are retained with exact transforms "
                "but disabled and resolved through five metadata-only YTYP identities."
            ),
        }
        (research_root / "import_receipt.json").write_text(
            json.dumps(receipt, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )
        (research_root / "transform_audit.json").write_text(
            json.dumps({
                "schema": BATCH_SCHEMA,
                "units_per_meter": converted.get("units_per_meter"),
                "basis_source_to_engine": converted.get("basis_source_to_engine"),
                "counts": converted.get("counts"),
                "transform_audit": converted.get("transform_audit"),
                "instances": [
                    {
                        "id": item["id"],
                        "reference_form_id": item.get("reference_form_id"),
                        "base_form_id": item.get("base_form_id"),
                        "cell_form_id": item.get("cell_form_id"),
                        "asset_id": item.get("asset_id"),
                        "source_models": item.get("source_models"),
                        "position": item["position"],
                        "rotation_ypr": item["rotation_ypr"],
                        "scale": item["scale"],
                        "enabled": item.get("enabled", True),
                    }
                    for item in placement_instances
                ],
            }, indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )

        # Publish only generated owner-scoped source trees; unrelated project files survive re-import.
        project_root.mkdir(parents=True, exist_ok=True)
        replace_generated_tree(
            model_source_root,
            project_root / "Source" / "models" / "maps" / map_id,
        )
        replace_generated_tree(
            definition_source_root,
            project_root / "Source" / "definitions" / "maps" / map_id,
        )
        (project_root / "Source" / "maps").mkdir(parents=True, exist_ok=True)
        shutil.copy2(map_source, project_root / "Source" / "maps" / map_source.name)
        target_research = project_root / "Source" / "Research" / "SanctuaryHillsPreWar"
        replace_generated_tree(research_root, target_research)
        shutil.copy2(staged_plan, project_root / "asset.build.json")

    plan_path = project_root / "asset.build.json"
    if not options.no_build:
        run_owner_pipeline(repo, plan_path)

    runtime_model_root = project_root / "Content" / "models" / "maps" / map_id
    runtime_definition_root = project_root / "Content" / "definitions" / "maps" / map_id
    runtime_map = project_root / "Content" / "maps" / f"{map_id}.ymap"
    runtime_models = len(list(runtime_model_root.glob("*.ydd"))) if runtime_model_root.is_dir() else 0
    runtime_definitions = len(list(runtime_definition_root.glob("*.ytyp"))) if runtime_definition_root.is_dir() else 0
    if not options.no_build:
        if runtime_models != 443:
            raise RuntimeError(f"runtime YDD count mismatch expected=443 actual={runtime_models}")
        if runtime_definitions != 448:
            raise RuntimeError(f"runtime YTYP count mismatch expected=448 actual={runtime_definitions}")
        if not runtime_map.is_file():
            raise RuntimeError(f"runtime YMAP was not published: {runtime_map}")

    print(
        "FALLOUT4_MAP_IMPORT_OK "
        f"project='{project_name}' map='maps/{map_id}.ymap' placements=4017 cells={cell_count} "
        f"assets=448 drawables=443 metadata_markers=5 disabled_markers=123 "
        f"transform_error={(converted.get('transform_audit') or {}).get('max_reconstructed_engine_matrix_error')} "
        f"build={not options.no_build}"
    )
    print(f"[PROJECT] {project_root}")
    print(f"[PLAN] {plan_path}")
    print(f"[MAP SOURCE] {project_root / 'Source' / 'maps' / f'{map_id}.ymap.xml'}")
    if not options.no_build:
        print(f"[RUNTIME MAP] {runtime_map}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
