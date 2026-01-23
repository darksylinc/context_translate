import bpy
import csv
import os
import random


class OBJECT_OT_import_animated_subs_text_objects_csv(bpy.types.Operator):
    """Import animated subtitles from a CSV file"""
    bl_idname = "object.import_animated_subs__text_objects_csv"
    bl_label = "Import Animated Subtitles from CSV"
    bl_options = {'REGISTER', 'UNDO'}

    filepath: bpy.props.StringProperty(
        name="File Path",
        description="Filepath used for importing the CSV",
        default="",
        maxlen=1024,
        subtype='FILE_PATH',
    )

    def get_or_create_material(self, mat_name):
        bg_material = bpy.data.materials.get(mat_name)

        if bg_material is None:
            bg_material = bpy.data.materials.new(name=mat_name)
            bg_material.use_nodes = True

            nodes = bg_material.node_tree.nodes
            links = bg_material.node_tree.links

            nodes.clear()

            emission = nodes.new(type="ShaderNodeEmission")
            output = nodes.new(type="ShaderNodeOutputMaterial")

            emission.inputs["Color"].default_value = (
                random.random(),
                random.random(),
                random.random(),
                1.0
            )

            links.new(emission.outputs["Emission"], output.inputs["Surface"])

        return bg_material

    def execute(self, context):
        camera = bpy.data.objects.get("Camera")
        if not camera:
            self.report({'ERROR'}, f"Camera not found")
            return {'CANCELLED'}

        action = bpy.data.actions.get("SubtitleAnim")
        if not action:
            self.report({'ERROR'}, f"Create an action named 'SubtitleAnim'.")
            return {'CANCELLED'}

        node_group_name = "Text Outliner S White"
        outline_geomnode = bpy.data.node_groups.get(node_group_name)
        if outline_geomnode is None:
            self.report(
                {'WARNING'}, f"Geometry Node '{node_group_name}' not found, ignoring.")

        italics_geomnode_name = "Italics (Shear)"
        italics_geomnode = bpy.data.node_groups.get(italics_geomnode_name)
        if italics_geomnode is None:
            self.report(
                {'WARNING'}, f"Geometry Node '{italics_geomnode_name}' not found, ignoring.")

        # Ensure collection exists
        coll_name = "English"
        if coll_name in bpy.data.collections:
            collection_target = bpy.data.collections[coll_name]
        else:
            collection_target = bpy.data.collections.new(coll_name)
            context.scene.collection.children.link(collection_target)

        imported_count = 0

        with open(self.filepath, newline='', encoding='utf-8') as csvfile:
            reader = csv.DictReader(csvfile, delimiter=';', quotechar='"')
            for row in reader:
                uid = row["UID"]
                speaker = row["Speaker"]
                use_italics = row['S'].find('I') != -1

                if len(speaker) == 0:
                    # This line is just a comment. Skip it.
                    continue

                obj_name = "Sub." + speaker + "." + str(uid)

                obj = bpy.data.objects.get(obj_name)
                if not obj:
                    text_data = bpy.data.curves.new(name=obj_name, type='FONT')
                    obj = bpy.data.objects.new(
                        name=obj_name, object_data=text_data)
                    collection_target.objects.link(obj)

                    mat = bpy.data.materials.get("White No Shadows")
                    if mat:
                        obj.data.materials.append(mat)

                    if outline_geomnode is not None:
                        bg_material = self.get_or_create_material(
                            speaker + " Subs")

                        # Add Geometry Node.
                        obj.modifiers.new(name='GeometryNodes', type='NODES')
                        new_mod = obj.modifiers[-1]
                        new_mod.node_group = outline_geomnode
                        new_mod['Socket_2'] = 0.002
                        new_mod['Socket_3'] = 1.005
                        new_mod['Socket_4'] = 1.005
                        new_mod['Socket_5'] = bg_material
                        obj.data.materials.append(bg_material)

                    if italics_geomnode is not None and use_italics:
                        obj.modifiers.new(name='GeometryNodes', type='NODES')
                        new_mod = obj.modifiers[-1]
                        new_mod.node_group = italics_geomnode
                        new_mod['Socket_2'] = 0.4

                if obj.type != 'FONT':
                    self.report(
                        {'WARNING'}, f"Object '{obj_name}' not TEXT. Ignoring.")
                    continue

                obj.data.body = row["Text"]
                obj.data.size = 0.056412
                obj.data.align_x = 'CENTER'
                obj.data.align_y = 'CENTER'

                obj.visible_diffuse = False
                obj.visible_glossy = False
                obj.visible_shadow = False
                obj.visible_transmission = False
                obj.visible_volume_scatter = False

                obj.location = camera.location
                obj.parent = camera

                frame_start = int(row["From"])
                frame_count = int(row["Length"])

                if obj.animation_data:
                    obj.animation_data_clear()
                obj.animation_data_create()
                nla_tracks = obj.animation_data.nla_tracks
                nla_track = nla_tracks.new()
                nla_track.name = "SubtitleAnim"
                nla_strip = nla_track.strips.new(
                    "SubtitleAnim", frame_start, action)
                nla_strip.scale = frame_count
                nla_strip.extrapolation = 'NOTHING'

                imported_count += 1

        self.report(
            {'INFO'}, f"Imported {imported_count} subtitle TEXT objects from {self.filepath}")
        return {'FINISHED'}

    def invoke(self, context, event):
        if not self.filepath:
            self.filepath = os.path.join(
                bpy.path.abspath("//"),
                "animated_subtitle_objects.csv"
            )
        context.window_manager.fileselect_add(self)
        return {'RUNNING_MODAL'}
