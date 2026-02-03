"""Studio Tools Blender Addon.

This addon provides pipeline integration for Blender:
- Asset browser integration
- Publish tools
- Scene validation
"""

bl_info = {
    "name": "Studio Tools",
    "author": "Studio Pipeline Team",
    "version": (1, 0, 0),
    "blender": (4, 2, 0),
    "location": "View3D > Sidebar > Studio",
    "description": "Pipeline tools and asset management",
    "category": "Pipeline",
}

import bpy


class STUDIO_PT_main_panel(bpy.types.Panel):
    """Main Studio Tools panel."""
    
    bl_label = "Studio Tools"
    bl_idname = "STUDIO_PT_main_panel"
    bl_space_type = 'VIEW_3D'
    bl_region_type = 'UI'
    bl_category = "Studio"

    def draw(self, context):
        layout = self.layout
        
        layout.label(text="Pipeline Tools", icon='LINKED')
        
        col = layout.column(align=True)
        col.operator("studio.publish_asset", text="Publish Asset", icon='EXPORT')
        col.operator("studio.validate_scene", text="Validate Scene", icon='CHECKMARK')
        
        layout.separator()
        
        layout.label(text="Quick Actions", icon='PLAY')
        row = layout.row(align=True)
        row.operator("studio.open_in_explorer", text="", icon='FILE_FOLDER')
        row.operator("studio.copy_path", text="", icon='COPYDOWN')


class STUDIO_OT_publish_asset(bpy.types.Operator):
    """Publish the current asset to the pipeline."""
    
    bl_idname = "studio.publish_asset"
    bl_label = "Publish Asset"
    bl_options = {'REGISTER', 'UNDO'}

    def execute(self, context):
        self.report({'INFO'}, "Publish functionality coming soon!")
        return {'FINISHED'}


class STUDIO_OT_validate_scene(bpy.types.Operator):
    """Validate the current scene against pipeline standards."""
    
    bl_idname = "studio.validate_scene"
    bl_label = "Validate Scene"
    bl_options = {'REGISTER'}

    def execute(self, context):
        errors = []
        warnings = []
        
        # Check for unnamed objects
        for obj in bpy.data.objects:
            if obj.name.startswith("Cube") or obj.name.startswith("Sphere"):
                warnings.append(f"Object '{obj.name}' has default name")
        
        # Check for unapplied transforms
        for obj in bpy.data.objects:
            if obj.type == 'MESH':
                if obj.scale != (1, 1, 1):
                    warnings.append(f"Object '{obj.name}' has unapplied scale")
        
        if errors:
            for e in errors:
                self.report({'ERROR'}, e)
            return {'CANCELLED'}
        
        if warnings:
            for w in warnings:
                self.report({'WARNING'}, w)
        else:
            self.report({'INFO'}, "Scene validation passed!")
        
        return {'FINISHED'}


class STUDIO_OT_open_in_explorer(bpy.types.Operator):
    """Open current file location in file browser."""
    
    bl_idname = "studio.open_in_explorer"
    bl_label = "Open in Explorer"

    def execute(self, context):
        import os
        import subprocess
        import sys
        
        filepath = bpy.data.filepath
        if not filepath:
            self.report({'WARNING'}, "File not saved yet")
            return {'CANCELLED'}
        
        dirpath = os.path.dirname(filepath)
        
        if sys.platform == 'darwin':
            subprocess.run(['open', dirpath])
        elif sys.platform == 'win32':
            subprocess.run(['explorer', dirpath])
        else:
            subprocess.run(['xdg-open', dirpath])
        
        return {'FINISHED'}


class STUDIO_OT_copy_path(bpy.types.Operator):
    """Copy current file path to clipboard."""
    
    bl_idname = "studio.copy_path"
    bl_label = "Copy Path"

    def execute(self, context):
        filepath = bpy.data.filepath
        if not filepath:
            self.report({'WARNING'}, "File not saved yet")
            return {'CANCELLED'}
        
        context.window_manager.clipboard = filepath
        self.report({'INFO'}, f"Copied: {filepath}")
        return {'FINISHED'}


classes = (
    STUDIO_PT_main_panel,
    STUDIO_OT_publish_asset,
    STUDIO_OT_validate_scene,
    STUDIO_OT_open_in_explorer,
    STUDIO_OT_copy_path,
)


def register():
    for cls in classes:
        bpy.utils.register_class(cls)
    print("Studio Tools addon registered")


def unregister():
    for cls in reversed(classes):
        bpy.utils.unregister_class(cls)
    print("Studio Tools addon unregistered")


if __name__ == "__main__":
    register()
