import bpy
from .exporter_csv import OBJECT_OT_export_text_objects_csv
from .importer_csv import OBJECT_OT_import_text_objects_csv
from .importer_animated_subs_csv import OBJECT_OT_import_animated_subs_text_objects_csv


bl_info = {
    "name": "CSV Text Importer/Exporter",
    "author":	"Matías	N. Goldberg",
    "version": (1, 0),
    "blender": (4, 2, 0),
    "category": "Import-Export",
    "location": "",
    "warning": "",
    "wiki_url": "",
    "tracker_url": "",
    "description": "Text exporter/importer for automated translators"
}


def menu_func_export(self, context):
    self.layout.operator(OBJECT_OT_export_text_objects_csv.bl_idname,
                         text="Export Text Objects to CSV")


def menu_func_import(self, context):
    self.layout.operator(OBJECT_OT_import_text_objects_csv.bl_idname,
                         text="Import Text Objects from CSV")


def menu_func_import_animated_subs(self, context):
    self.layout.operator(OBJECT_OT_import_animated_subs_text_objects_csv.bl_idname,
                         text="Import Animated Subtitles from CSV")


def register():
    bpy.utils.register_class(OBJECT_OT_export_text_objects_csv)
    bpy.types.TOPBAR_MT_file_export.append(menu_func_export)
    bpy.utils.register_class(OBJECT_OT_import_text_objects_csv)
    bpy.types.TOPBAR_MT_file_import.append(menu_func_import)
    bpy.utils.register_class(OBJECT_OT_import_animated_subs_text_objects_csv)
    bpy.types.TOPBAR_MT_file_import.append(menu_func_import_animated_subs)


def unregister():
    bpy.types.TOPBAR_MT_file_import.remove(menu_func_import_animated_subs)
    bpy.utils.unregister_class(OBJECT_OT_import_animated_subs_text_objects_csv)
    bpy.types.TOPBAR_MT_file_import.remove(menu_func_import)
    bpy.utils.unregister_class(OBJECT_OT_import_text_objects_csv)
    bpy.types.TOPBAR_MT_file_export.remove(menu_func_export)
    bpy.utils.unregister_class(OBJECT_OT_export_text_objects_csv)


if __name__ == "__main__":
    register()
