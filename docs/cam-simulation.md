# CAM simulation

SolidCraft's CAM story starts with the tool library (`solidcraft-tooling`, the Cam
tab's Tool Library window): typed milling tools with exact revolved profiles. The
simulator answers the first question every machinist asks about a toolpath: **does this
path hit what it should not?** — the part surface (a gouge), the stock during a rapid
move (a crash) — and does it leave material behind?

This page records the design and its boundaries; the math lives in
`solidcraft_tooling::sim` and the command surface is `cam.simulate`.

## The model: Z-map stock and part

The simulator keeps two heightfields (Z-maps) over one rectangular XY grid:

- **stock** — the current material height in every cell, starting at the stock top;
- **part** — the finished height, the surface the toolpath is meant to produce.

A toolpath is a list of cutter-location (CL) points — the tool **tip** position plus
whether the move into the point cuts. Each move is sampled at half a cell; at every
sample the cutter's cutting surface lowers the stock inside its radius:

```text
for each sample (x, y, z):
    for each cell centre (cx, cy) within the tool radius:
        h = profile.cut_height_at(dist((x, y), (cx, cy)))   # surface height above the tip
        stock[cell] = min(stock[cell], z + h)               # material cannot grow back
```

`cut_height_at` is the inverse of the profile's `radius_at`: the lowest cutting-surface
height at a radial distance. Because it is derived from the same revolved profile the
previews render, every family — flat, ball, bull, tapered, drill, chamfer — is simulated
with its exact geometry, not an approximation of it.

Three verdicts come out of a replay:

| finding | meaning |
| --- | --- |
| **gouge** | a cut lands below the part surface by more than the tolerance; the report keeps the count, the worst depth and the first points |
| **remaining** | after the last move a cell still stands above the part — material left behind |
| **collision** | a rapid (non-cutting) move dips the tool's full envelope below the stock — a crash, not a cut |

Removed volume is accumulated as the stock surface drops, so the report also carries a
milligram-scale `removed_mm3`.

## Why a Z-map

The classic alternatives are dexels (multi-directional heightfields), ACLx adaptive cut
levels, and voxel octrees. For the families in the tool library — everything is a
revolved profile plunging or sweeping in Z — a single vertical Z-map is:

- **exact** for vertical-wall 2.5-D work, which is what these tools do;
- **analytic** — gouge depths are real millimetres, not voxel staircases;
- **cheap** — one `f64` per cell, no boolean mesh per cut;
- **hostile-input friendly** — grid, path length and a total work budget are all capped
  before a single sample runs, so a bad file cannot spend unbounded time or memory.

The known boundary: undercuts, overhangs and steep 5-axis walls are outside a vertical
heightfield. When 3+2 or 5-axis work arrives, the same profile queries drive a dexel or
ACLx core; the command surface (`cam.simulate` inputs and report) is meant to stay
stable across that swap.

## Command surface

`cam.simulate` (headless, `doc_only`, nothing in the design changes):

```json
{"command": "cam.simulate", "params": {
  "family": "end_mill_flat",
  "params": {"cutting_diameter": 10, "flute_length": 20, "overall_length": 70, "shank_diameter": 10},
  "path": [[5, 20, 12], [5, 20, 5], {"x": 35, "y": 20, "z": 5}],
  "stock": {"min": [0, 0, 0], "max": [40, 40, 10]},
  "part_top": 5,
  "cell": 0.5,
  "tolerance": 0.05,
  "unit": "mm"
}}
```

- `path` points are `[x, y, z]` or `{x, y, z, cut?}`; the first point is a position, and
  a move cuts unless it says `cut: false` (a rapid).
- `part_top` defaults to the stock top, which turns the run into a pure "don't touch
  anything" check.
- `cell` defaults to the footprint's long side / 200, clamped to the grid limits.
- The report: `removed_mm3`, `gouges`, `gouge_worst_mm`, `gouge_points`,
  `remaining_cells`, `remaining_worst_mm`, `collisions`, `samples`, `grid`.

Everything is bounded: at most 384×384 cells, 100 000 path points, and a combined
sample × cell-touch work budget that refuses a too-fine path up front instead of
grinding for minutes.

## Roadmap

1. **G-code / CLSF reader** so a post-processed file can be replayed directly
   (`cam.simulate` already takes the same shape a CL file decodes to).
2. **Part from the design**: take the active body's tessellation and build the part
   Z-map from it, instead of a flat `part_top`.
3. **A CAM page**: stock/part preview with gouge cells highlighted, driven by the same
   report; camera through `navigation::handle` like every other canvas.
4. **Dexel core** for 3+2 and 5-axis work behind the same command.
