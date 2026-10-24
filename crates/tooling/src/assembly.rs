//! Tool assemblies: a cutter seated in a catalogue holder at a given stickout.
//!
//! The tool stands with its tip at z = 0; the holder's girth line sits at z = `stickout`, so the
//! holder profile (which spans −nose_len..+taper_len locally) is translated up by the stickout
//! and merged with the tool mesh. Faces keep their roles — cutting gold, shank silver, holder
//! gray — so a viewer paints the three parts apart without knowing the families.

use crate::mesh::{self, Revolved, Role};
use crate::profile::CutterProfile;
use crate::{Result, holder, invalid};

/// A cutter in a holder: the merged mesh plus the lengths CAM consumers need.
pub struct Assembly {
    pub mesh: Revolved,
    /// Tool tip to the top of the holder taper.
    pub overall_length: f64,
    /// Largest radius of either part.
    pub max_radius: f64,
    /// How far the tip protrudes below the holder nose (`stickout − nose length`).
    pub reach: f64,
}

/// Compose `tool` in the catalogue holder `holder_type` gripping `bore` (the shank diameter),
/// with the girth line `stickout` above the tip, revolved at `steps` angular steps.
pub fn compose(tool: &CutterProfile, holder_type: &str, bore: f64, stickout: f64, steps: usize) -> Result<Assembly> {
    let meta = holder::find(holder_type).ok_or_else(|| invalid(format!("unknown holder type `{holder_type}`")))?;
    if !stickout.is_finite() || stickout <= meta.nose_len {
        return Err(invalid(format!("stickout must be greater than the holder nose length ({} mm)", meta.nose_len)));
    }
    let holder_profile = holder::profile_of(holder_type, bore)?;
    let mut tool_rev = mesh::revolve(tool, steps)?;
    let holder_rev = mesh::revolve_with(&holder_profile, steps, Role::Holder)?;
    let mut holder_mesh = holder_rev.mesh;
    for p in &mut holder_mesh.positions {
        p.z += stickout;
    }
    // Rebase the holder's vertex indices and face ids into the merged mesh.
    let pos_offset = tool_rev.mesh.positions.len() as u32;
    for tri in &mut holder_mesh.triangles {
        for v in tri.iter_mut() {
            *v += pos_offset;
        }
    }
    let face_offset = tool_rev.face_roles.len() as u32;
    for f in &mut holder_mesh.tri_face {
        *f += face_offset;
    }
    tool_rev.mesh.positions.extend(holder_mesh.positions);
    tool_rev.mesh.normals.extend(holder_mesh.normals);
    tool_rev.mesh.triangles.extend(holder_mesh.triangles);
    tool_rev.mesh.tri_face.extend(holder_mesh.tri_face);
    tool_rev.face_roles.extend(holder_rev.face_roles);
    Ok(Assembly {
        overall_length: stickout + meta.overall_length - meta.nose_len,
        max_radius: tool.max_radius.max(holder_profile.max_radius),
        reach: stickout - meta.nose_len,
        mesh: tool_rev,
    })
}

#[cfg(test)]
#[path = "assembly_tests.rs"]
mod tests;
