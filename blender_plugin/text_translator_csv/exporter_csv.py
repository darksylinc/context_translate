from mathutils import Vector
from functools import cmp_to_key
import bpy
import csv
import os


def get_aabb(obj):
    """Return world-space AABB min/max for the object"""
    coords = [obj.matrix_world @ Vector(corner) for corner in obj.bound_box]
    min_corner = Vector((min(v.x for v in coords),
                         min(v.y for v in coords),
                         min(v.z for v in coords)))
    max_corner = Vector((max(v.x for v in coords),
                         max(v.y for v in coords),
                         max(v.z for v in coords)))
    return min_corner, max_corner


def roughly_same_height(minA, maxA, minB, maxB, epsilon=0.1):
    upperA = maxA.z + epsilon
    lowerA = minA.z - epsilon
    upperB = maxB.z + epsilon
    lowerB = minB.z - epsilon

    return not (lowerA < upperB or upperA > lowerB)


def cmp_sort_by_pos(a, b):
    minA, maxA = get_aabb(a)
    minB, maxB = get_aabb(b)

    if roughly_same_height(minA, maxA, minB, maxB):
        return -1 if maxA.x > maxB.x else (1 if maxA.x < maxB.x else 0)
    else:
        return -1 if minA.z > minB.z else (1 if minA.z < minB.z else 0)


def sort_text_objects(text_objects):
    return sorted(text_objects, key=cmp_to_key(cmp_sort_by_pos))


class OBJECT_OT_export_text_objects_csv(bpy.types.Operator):
    """Export all TEXT objects into a CSV file"""
    bl_idname = "object.export_text_objects_csv"
    bl_label = "Export Text Objects to CSV"
    bl_options = {'REGISTER', 'UNDO'}

    filepath: bpy.props.StringProperty(
        name="File Path",
        description="Filepath used for exporting the CSV",
        default="",
        maxlen=1024,
        subtype='FILE_PATH',
    )

    def execute(self, context):
        text_objects = [
            obj for obj in context.view_layer.objects if obj.type == 'FONT' and obj.visible_get()]

        text_objects = sort_text_objects(text_objects)

        # Open CSV file
        with open(self.filepath, 'w', newline='', encoding='utf-8') as csvfile:
            writer = csv.writer(csvfile, delimiter=';',
                                quotechar='"', quoting=csv.QUOTE_ALL)

            # Header
            writer.writerow(["datablock_name", "Collection", "Text Contents"])

            for obj in text_objects:
                # Get first collection name (if any)
                col_name = obj.users_collection[0].name if obj.users_collection else ""

                # Write row
                writer.writerow([obj.name, col_name, obj.data.body])

        self.report(
            {'INFO'}, f"Exported {len(text_objects)} TEXT objects to {self.filepath}")
        return {'FINISHED'}

    def invoke(self, context, event):
        if not self.filepath:
            self.filepath = os.path.join(
                bpy.path.abspath("//"),
                "text_objects.csv"
            )
        context.window_manager.fileselect_add(self)
        return {'RUNNING_MODAL'}
