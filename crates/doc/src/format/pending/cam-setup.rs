//! A design can carry a CAM setup: operations with their tool, stock and 3D LINE/ARC
//! toolpath, fixtures and a work origin, under a `cam` member (the `solidcraft-cam`
//! crate). Older designs need no transformation: they have no CAM, and the member
//! defaults to empty. The last release refuses a design saved with this (the newer
//! format number).
