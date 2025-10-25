import bpy
import csv
import os


class OBJECT_OT_import_text_objects_csv(bpy.types.Operator):
    """Import translated TEXT objects from a CSV file and duplicate originals"""
    bl_idname = "object.import_text_objects_csv"
    bl_label = "Import Text Objects from CSV"
    bl_options = {'REGISTER', 'UNDO'}

    filepath: bpy.props.StringProperty(
        name="File Path",
        description="Filepath used for importing the CSV",
        default="",
        maxlen=1024,
        subtype='FILE_PATH',
    )

    def execute(self, context):
        # Ensure "Japanese Text" collection exists
        coll_name = "Japanese Text"
        if coll_name in bpy.data.collections:
            jp_collection = bpy.data.collections[coll_name]
        else:
            jp_collection = bpy.data.collections.new(coll_name)
            context.scene.collection.children.link(jp_collection)

        imported_count = 0

        with open(self.filepath, newline='', encoding='utf-8') as csvfile:
            reader = csv.DictReader(csvfile, delimiter=';', quotechar='"')
            for row in reader:
                datablock_name = row["datablock_name"]
                new_text = row["Text Contents"]

                obj = bpy.data.objects.get(datablock_name)
                if not obj or obj.type != 'FONT':
                    self.report(
                        {'WARNING'}, f"Object '{datablock_name}' not found or not TEXT")
                    continue

                # Duplicate object and its datablock
                new_obj = obj.copy()
                new_obj.data = obj.data.copy()

                """
                # Duplicate animation data (if any)
                if obj.animation_data:
                    new_obj.animation_data_create()
                    new_obj.animation_data.action = obj.animation_data.action

                # Duplicate modifiers
                for mod in obj.modifiers:
                    new_obj.modifiers.new(name=mod.name, type=mod.type)
                    new_mod = new_obj.modifiers[-1]
                    for attr in dir(mod):
                        # Copy simple attributes only
                        if not attr.startswith("_") and not callable(getattr(mod, attr)):
                            try:
                                setattr(new_mod, attr, getattr(mod, attr))
                            except Exception:
                                pass

                # Duplicate constraints
                for con in obj.constraints:
                    new_con = new_obj.constraints.new(type=con.type)
                    for attr in dir(con):
                        if not attr.startswith("_") and not callable(getattr(con, attr)):
                            try:
                                setattr(new_con, attr, getattr(con, attr))
                            except Exception:
                                pass
                """

                # Rename object and datablock
                new_obj.name = datablock_name + "_jp"
                new_obj.data.name = obj.data.name + "_jp"

                new_obj.data.resolution_u = 12
                new_obj.data.font = bpy.data.fonts["Bfont Regular"]

                # Set translated text
                new_obj.data.body = new_text

                # Link to "Japanese" collection
                jp_collection.objects.link(new_obj)

                imported_count += 1

        self.report(
            {'INFO'}, f"Imported {imported_count} translated TEXT objects from {self.filepath}")
        return {'FINISHED'}

    def invoke(self, context, event):
        if not self.filepath:
            self.filepath = os.path.join(
                bpy.path.abspath("//"),
                "translated_text_objects.csv"
            )
        context.window_manager.fileselect_add(self)
        return {'RUNNING_MODAL'}
