#!/usr/bin/env python3
"""Blender-side Fallout 4 NIF batch exporter for North Star maps.

Reads a placement manifest produced from Fallout4.esm, imports each unique NIF
through PyNifly in source/Bethesda coordinates, bakes the NIF node hierarchy
into one reusable OBJ, and converts all placement transforms into North Star's
Y-up coordinate system.

The conversion deliberately does not use PyNifly's 0.1 authoring display scale.
North Star uses meter-scale gameplay coordinates, so Bethesda world units are
normalized with --units-per-meter (default 70).
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
import sys
import traceback
from pathlib import Path

import bpy
from mathutils import Euler, Matrix, Vector


def parse_args() -> argparse.Namespace:
    argv = sys.argv
    argv = argv[argv.index("--") + 1:] if "--" in argv else []
    parser = argparse.ArgumentParser()
    parser.add_argument("--source-manifest", required=True, type=Path)
    parser.add_argument("--data-root", required=True, type=Path)
    parser.add_argument("--geometry-dir", required=True, type=Path)
    parser.add_argument("--result-manifest", required=True, type=Path)
    parser.add_argument("--pynifly-root", required=True, type=Path)
    parser.add_argument(
        "--material-logical-root",
        default="materials/fallout4",
        help="Native logical NEMAT root used by emitted OBJ usemtl selectors.",
    )
    parser.add_argument("--units-per-meter", type=float, default=70.0)
    parser.add_argument("--limit-assets", type=int)
    return parser.parse_args(argv)


def slug(value: str) -> str:
    value = re.sub(r"[^a-zA-Z0-9_]+", "_", value.strip()).strip("_").lower()
    return value or "mesh"


def asset_id_for_model(model: str) -> str:
    normalized = model.replace("\\", "/").strip().lower()
    stem = slug(Path(normalized).stem)
    digest = hashlib.sha1(normalized.encode("utf-8")).hexdigest()[:10]
    return f"{stem}_{digest}"


def canonical_model(model: str) -> str:
    return model.replace("/", "\\").lstrip("\\").strip()


def resolve_model_path(data_root: Path, model: str) -> Path:
    model = canonical_model(model)
    candidates = [
        data_root / "Meshes" / Path(model.replace("\\", "/")),
        data_root / "meshes" / Path(model.replace("\\", "/")),
        data_root / Path(model.replace("\\", "/")),
    ]
    for candidate in candidates:
        if candidate.is_file():
            return candidate.resolve()
    # Windows is case-insensitive normally, but keep a bounded case-insensitive
    # component walk for extracted packages created on case-sensitive volumes.
    root = data_root / "Meshes"
    current = root
    if root.is_dir():
        for part in Path(model.replace("\\", "/")).parts:
            if not current.is_dir():
                break
            match = next((p for p in current.iterdir() if p.name.casefold() == part.casefold()), None)
            if match is None:
                break
            current = match
        if current.is_file():
            return current.resolve()
    raise FileNotFoundError(f"NIF not found for model '{model}' under '{data_root}'")


def source_to_engine_basis(units_per_meter: float) -> Matrix:
    # Fallout/Bethesda source is right-handed Z-up. North Star is right-handed
    # Y-up, matching the existing Blender map exporter conversion:
    #   FO4 (x,y,z) -> NS (x,z,-y)
    # Do NOT include PyNifly's extra 180-degree Blender display rotation here;
    # it is an authoring convenience, not part of Creation Engine world space.
    s = 1.0 / units_per_meter
    return Matrix((
        (s, 0.0, 0.0, 0.0),
        (0.0, 0.0, s, 0.0),
        (0.0, -s, 0.0, 0.0),
        (0.0, 0.0, 0.0, 1.0),
    ))


def source_matrix(instance: dict) -> Matrix:
    position = instance.get("position_bethesda_units") or [0.0, 0.0, 0.0]
    rotation = instance.get("rotation_radians_xyz") or [0.0, 0.0, 0.0]
    scale = float(instance.get("scale", 1.0))
    if len(position) != 3 or len(rotation) != 3:
        raise ValueError(f"invalid transform on REFR {instance.get('reference_form_id')}")
    if not all(math.isfinite(float(x)) for x in [*position, *rotation, scale]):
        raise ValueError(f"non-finite transform on REFR {instance.get('reference_form_id')}")
    rx, ry, rz = (float(v) for v in rotation)
    # Creation Engine REFR angles are clockwise-positive and applied Z, then Y,
    # then X. With column vectors this is the transpose of naive Rz @ Ry @ Rx.
    # Using the engine convention here avoids the classic "plausible but mirrored
    # interior" failure mode.
    rot = (
        Matrix.Rotation(rz, 4, "Z")
        @ Matrix.Rotation(ry, 4, "Y")
        @ Matrix.Rotation(rx, 4, "X")
    ).transposed()
    return (
        Matrix.Translation(Vector(tuple(float(v) for v in position)))
        @ rot
        @ Matrix.Diagonal((scale, scale, scale, 1.0))
    )


def placement_payload(instance: dict, basis: Matrix) -> tuple[dict, float]:
    src = source_matrix(instance)
    engine = basis @ src @ basis.inverted()
    location, rotation, scale = engine.decompose()
    euler = rotation.to_euler("YXZ")
    payload = {
        "position": [float(location.x), float(location.y), float(location.z)],
        "rotation_ypr": [float(euler.y), float(euler.x), float(euler.z)],
        "scale": [float(scale.x), float(scale.y), float(scale.z)],
    }

    # Round-trip through decomposed runtime semantics and compare matrices.
    # The YMAP convention stores yaw/pitch/roll via a YXZ extraction. Rebuild
    # using the same extraction order rather than assuming source XYZ.
    reconstructed = (
        Matrix.Translation(location)
        @ Euler((float(euler.x), float(euler.y), float(euler.z)), "YXZ").to_matrix().to_4x4()
        @ Matrix.Diagonal((float(scale.x), float(scale.y), float(scale.z), 1.0))
    )
    error = max(abs(float(engine[r][c] - reconstructed[r][c])) for r in range(4) for c in range(4))
    return payload, error


def configure_pynifly(root: Path, data_root: Path) -> None:
    root = root.resolve()
    data_root = data_root.resolve()
    if str(root) not in sys.path:
        sys.path.insert(0, str(root))
    import io_scene_nifly
    try:
        io_scene_nifly.unregister()
    except Exception:
        pass
    io_scene_nifly.register()
    if not hasattr(bpy.ops.import_scene, "pynifly"):
        raise RuntimeError("PyNifly import operator did not register")

    # Headless FO4 import needs the authoritative extracted Data root, but it does
    # not need Blender image/shader construction. Keep PyNifly's shape -> material
    # association while recording only the source BGSM/BGEM identity + texture set.
    # The native NorthStar pass translates those records into NEMAT/YTD.
    from io_scene_nifly.nif import shader_io
    shader_io.ShaderImporter._build_alt_pathlist_for_game = staticmethod(
        lambda _game: [str(data_root)]
    )

    def import_material_identity(self, obj, shape, _asset_path):
        if getattr(obj, "type", "") == "EMPTY":
            return
        try:
            shape.shader.alternate_paths = [str(data_root)]
        except Exception:
            pass
        material_ref = str(getattr(shape.shader, "name", "") or "").replace("\\", "/")
        try:
            textures = {str(k): str(v).replace("\\", "/") for k, v in shape.textures.items() if v}
        except Exception:
            textures = {}
        obj["northstar_fo4_material_ref"] = material_ref
        obj["northstar_fo4_textures"] = json.dumps(textures, ensure_ascii=False, sort_keys=True)

    shader_io.ShaderImporter.import_material = import_material_identity


def reset_scene() -> None:
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for datablocks in (
        bpy.data.meshes,
        bpy.data.curves,
        bpy.data.armatures,
        bpy.data.materials,
        bpy.data.images,
        bpy.data.cameras,
        bpy.data.lights,
    ):
        for block in list(datablocks):
            if block.users == 0:
                datablocks.remove(block)


def is_render_mesh(obj: bpy.types.Object) -> bool:
    if obj.type != "MESH" or obj.hide_render:
        return False
    if obj.get("FO4_CUTPOINT"):
        return False
    if obj.get("pynMultiBoundOBB"):
        return False
    name = obj.name.casefold()
    if name.endswith(":bbx") or "cutpoint " in name:
        return False
    return True


def safe_group_name(value: str) -> str:
    value = re.sub(r"\s+", "_", value.strip())
    value = re.sub(r"[^A-Za-z0-9_.-]+", "_", value)
    return value or "mesh"


def canonical_material_ref(value: str) -> str:
    value = str(value or "").replace("\\", "/").strip().lstrip("/")
    low = value.casefold()
    marker = low.find("materials/")
    if marker >= 0:
        value = value[marker:]
    elif value and not low.startswith("materials/"):
        value = "Materials/" + value
    return value


def material_record_for_object(obj: bpy.types.Object, logical_root: str) -> dict:
    source_ref = canonical_material_ref(obj.get("northstar_fo4_material_ref", ""))
    try:
        textures = json.loads(str(obj.get("northstar_fo4_textures", "{}")))
    except Exception:
        textures = {}
    textures = {str(k): str(v).replace("\\", "/") for k, v in textures.items() if v}
    identity = source_ref.casefold() if source_ref else json.dumps(textures, sort_keys=True).casefold()
    if not identity:
        identity = "untextured:" + obj.name.casefold()
    digest = hashlib.sha1(identity.encode("utf-8")).hexdigest()[:12]
    stem = slug(Path(source_ref).stem) if source_ref else "inline"
    material_id = f"{stem}_{digest}"
    root = logical_root.replace("\\", "/").strip("/")
    selector = f"{root}/{material_id}.nemat@material"
    return {
        "material_id": material_id,
        "selector": selector,
        "source_material_ref": source_ref,
        "textures": textures,
    }


def write_nif_obj(
    path: Path,
    asset_id: str,
    mesh_objects: list[bpy.types.Object],
    basis: Matrix,
    material_catalog: dict[str, dict],
    material_logical_root: str,
) -> dict:
    path.parent.mkdir(parents=True, exist_ok=True)
    depsgraph = bpy.context.evaluated_depsgraph_get()
    lines: list[str] = [
        "# North Star Fallout 4 NIF conversion",
        f"o {asset_id}",
    ]
    vertex_base = 0
    uv_base = 0
    total_vertices = 0
    total_triangles = 0
    bounds_min = [math.inf, math.inf, math.inf]
    bounds_max = [-math.inf, -math.inf, -math.inf]

    for source in sorted(mesh_objects, key=lambda o: o.name.casefold()):
        evaluated = source.evaluated_get(depsgraph)
        mesh = evaluated.to_mesh()
        if mesh is None:
            continue
        try:
            mesh.calc_loop_triangles()
            if not mesh.vertices or not mesh.loop_triangles:
                continue
            material_record = material_record_for_object(source, material_logical_root)
            material_catalog.setdefault(material_record["selector"], material_record)
            lines.append(f"g {safe_group_name(source.name)}")
            lines.append(f"usemtl {material_record['selector']}")
            world = evaluated.matrix_world.copy()
            for vertex in mesh.vertices:
                p = basis @ (world @ vertex.co.to_4d())
                xyz = [float(p.x), float(p.y), float(p.z)]
                for axis in range(3):
                    bounds_min[axis] = min(bounds_min[axis], xyz[axis])
                    bounds_max[axis] = max(bounds_max[axis], xyz[axis])
                lines.append(f"v {xyz[0]:.9g} {xyz[1]:.9g} {xyz[2]:.9g}")
            uv_data = mesh.uv_layers.active.data if mesh.uv_layers.active is not None else None
            uv_lines: list[str] = []
            face_lines: list[str] = []
            local_uv_count = 0
            for tri in mesh.loop_triangles:
                tokens = []
                for vertex_index, loop_index in zip(tri.vertices, tri.loops):
                    vi = vertex_base + int(vertex_index) + 1
                    if uv_data is not None:
                        uv = uv_data[loop_index].uv
                        local_uv_count += 1
                        ti = uv_base + local_uv_count
                        uv_lines.append(f"vt {float(uv.x):.9g} {float(uv.y):.9g}")
                        tokens.append(f"{vi}/{ti}")
                    else:
                        tokens.append(str(vi))
                face_lines.append("f " + " ".join(tokens))
            lines.extend(uv_lines)
            lines.extend(face_lines)
            vertex_base += len(mesh.vertices)
            uv_base += local_uv_count
            total_vertices += len(mesh.vertices)
            total_triangles += len(mesh.loop_triangles)
        finally:
            evaluated.to_mesh_clear()

    if total_vertices == 0 or total_triangles == 0:
        raise RuntimeError(f"NIF produced no renderable triangles for asset '{asset_id}'")
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return {
        "vertices": total_vertices,
        "triangles": total_triangles,
        "bounds_min": bounds_min,
        "bounds_max": bounds_max,
    }


def import_nif(filepath: Path) -> list[bpy.types.Object]:
    before = {obj.as_pointer() for obj in bpy.context.scene.objects}
    result = bpy.ops.import_scene.pynifly(
        filepath=str(filepath),
        blender_xf=False,
        import_collisions=False,
        import_animations=False,
        import_tris=False,
        import_cutpoints=False,
        rename_bones=False,
        rename_bones_niftools=False,
        rotate_bones_pretty=False,
        create_bones=False,
        create_collection=False,
        smart_editor_markers=True,
        import_shapekeys=False,
    )
    if "FINISHED" not in result:
        raise RuntimeError(f"PyNifly import failed: {filepath}")
    imported = [obj for obj in bpy.context.scene.objects if obj.as_pointer() not in before]

    # Creation Engine placement replaces the transform of every parentless NIF
    # root with the REFR transform. Keeping an authored transform on block 0 and
    # then applying REFR would double-transform affected statics. PyNifly marks
    # the imported file root with pynRoot, so normalize it before baking.
    for obj in imported:
        if obj.get("pynRoot"):
            obj.matrix_local = Matrix.Identity(4)
    bpy.context.view_layer.update()

    return [obj for obj in imported if is_render_mesh(obj)]


def editor_only_model(model: str) -> bool:
    normalized = model.replace("/", "\\").casefold()
    basename = normalized.rsplit("\\", 1)[-1]
    return (
        normalized.startswith("markers\\")
        or "\\editormarkers\\" in normalized
        or basename in {
            "markerxheading.nif",
            "xmarker.nif",
            "markerx.nif",
            "markercocheading.nif",
            "markercoheading.nif",
        }
    )


def main() -> int:
    options = parse_args()
    if not math.isfinite(options.units_per_meter) or options.units_per_meter <= 0:
        raise SystemExit("--units-per-meter must be finite and > 0")
    source = json.loads(options.source_manifest.read_text(encoding="utf-8"))
    instances = source.get("instances") or []
    if int(source.get("instance_count", len(instances))) != len(instances):
        raise RuntimeError("visual_instances instance_count does not match instances array")

    configure_pynifly(options.pynifly_root, options.data_root)
    basis = source_to_engine_basis(options.units_per_meter)
    options.geometry_dir.mkdir(parents=True, exist_ok=True)

    unique_models: dict[str, str] = {}
    for instance in instances:
        models = instance.get("models") or []
        for raw in models:
            model = canonical_model(str(raw))
            if model:
                unique_models.setdefault(model.casefold(), model)
    models = [unique_models[k] for k in sorted(unique_models)]
    if options.limit_assets is not None:
        models = models[: max(0, options.limit_assets)]

    assets = []
    failures = []
    material_catalog: dict[str, dict] = {}
    reset_scene()
    for index, model in enumerate(models, 1):
        asset_id = asset_id_for_model(model)
        try:
            nif_path = resolve_model_path(options.data_root, model)
            editor_only = editor_only_model(model)
            if editor_only:
                # Preserve editor-marker identity as a native metadata definition,
                # but never spend runtime geometry on Creation Kit-only helpers.
                # Their YMAP placements remain present and disabled.
                assets.append({
                    "asset_id": asset_id,
                    "model_path": model,
                    "nif_path": str(nif_path),
                    "geometry_file": "",
                    "material_ref": "",
                    "collision": False,
                    "editor_only": True,
                    "vertices": 0,
                    "triangles": 0,
                    "bounds_min": None,
                    "bounds_max": None,
                })
                print(
                    "FO4_NIF_EXPORT_MARKER "
                    f"{index}/{len(models)} asset={asset_id} model={model!r}"
                )
            else:
                mesh_objects = import_nif(nif_path)
                geometry_file = options.geometry_dir / f"{asset_id}.obj"
                stats = write_nif_obj(
                    geometry_file,
                    asset_id,
                    mesh_objects,
                    basis,
                    material_catalog,
                    options.material_logical_root,
                )
                assets.append({
                    "asset_id": asset_id,
                    "model_path": model,
                    "nif_path": str(nif_path),
                    "geometry_file": str(geometry_file),
                    "material_ref": "",
                    "collision": False,
                    "editor_only": False,
                    **stats,
                })
                print(
                    "FO4_NIF_EXPORT_OK "
                    f"{index}/{len(models)} asset={asset_id} meshes={len(mesh_objects)} "
                    f"verts={stats['vertices']} tris={stats['triangles']}"
                )
        except Exception as error:
            failures.append({
                "model_path": model,
                "asset_id": asset_id,
                "error": f"{type(error).__name__}: {error}",
                "traceback": traceback.format_exc(),
            })
            print(f"FO4_NIF_EXPORT_FAIL asset={asset_id} model={model!r}: {error}", file=sys.stderr)
        finally:
            reset_scene()

    model_to_asset = {item["model_path"].casefold(): item["asset_id"] for item in assets}
    converted_instances = []
    max_matrix_error = 0.0
    multi_model_instances = 0
    missing_model_instances = 0
    for item in instances:
        raw_models = [canonical_model(str(v)) for v in (item.get("models") or []) if str(v).strip()]
        if len(raw_models) > 1:
            multi_model_instances += 1
        resolved_assets = [model_to_asset[m.casefold()] for m in raw_models if m.casefold() in model_to_asset]
        if not resolved_assets:
            missing_model_instances += 1
            # Keep the source transform in the audit even when geometry failed.
            resolved_asset = ""
        else:
            # Current ESM extraction records the render MODL path first. Preserve
            # all paths in source_models and refuse multi-model ambiguity at host level.
            resolved_asset = resolved_assets[0]
        transform, matrix_error = placement_payload(item, basis)
        max_matrix_error = max(max_matrix_error, matrix_error)
        ref = str(item.get("reference_form_id") or "")
        editor_only = bool(raw_models) and all(editor_only_model(m) for m in raw_models)
        converted_instances.append({
            "id": f"fo4_{ref.casefold()}" if ref else f"fo4_ref_{len(converted_instances):08d}",
            "reference_form_id": ref,
            "base_form_id": item.get("base_form_id"),
            "base_editor_id": item.get("base_editor_id"),
            "base_record_type": item.get("base_record_type"),
            "cell_form_id": item.get("cell_form_id"),
            "source_models": raw_models,
            "asset_id": resolved_asset,
            "definition_ref": "",
            "apply_mode": "instantiate",
            "enabled": not editor_only,
            "tags": sorted(set([
                "fallout4",
                "sanctuary_prewar",
                str(item.get("base_record_type") or "unknown").casefold(),
                *(["fo4_editor_marker"] if editor_only else []),
            ])),
            "cell_override": None,
            **transform,
        })

    result = {
        "schema": "northstar.fallout4_nif_batch_export.v1",
        "source_manifest": str(options.source_manifest.resolve()),
        "data_root": str(options.data_root.resolve()),
        "units_per_meter": options.units_per_meter,
        "basis_source_to_engine": [[float(basis[r][c]) for c in range(4)] for r in range(4)],
        "assets": assets,
        "materials": sorted(material_catalog.values(), key=lambda item: item["selector"].casefold()),
        "instances": converted_instances,
        "counts": {
            "source_instances": len(instances),
            "converted_instances": len(converted_instances),
            "unique_source_models": len(unique_models),
            "attempted_assets": len(models),
            "converted_assets": len(assets),
            "render_assets": sum(1 for asset in assets if not asset.get("editor_only")),
            "metadata_marker_assets": sum(1 for asset in assets if asset.get("editor_only")),
            "failed_assets": len(failures),
            "source_materials": len(material_catalog),
            "multi_model_instances": multi_model_instances,
            "instances_without_converted_model": missing_model_instances,
        },
        "transform_audit": {
            "max_reconstructed_engine_matrix_error": max_matrix_error,
        },
        "failures": failures,
    }
    options.result_manifest.parent.mkdir(parents=True, exist_ok=True)
    options.result_manifest.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(
        "NORTHSTAR_FO4_NIF_BATCH_EXPORT_DONE "
        f"assets={len(assets)}/{len(models)} instances={len(converted_instances)} "
        f"failures={len(failures)} transform_error={max_matrix_error:.3e}"
    )
    return 0 if not failures else 2


if __name__ == "__main__":
    raise SystemExit(main())
