# Ideas

Kept because they were worth writing down, not because they are planned.

## Transitions

The scene needs to change over four and a half minutes. Each of these reuses
something already built rather than adding a new subsystem.

**Fluid dissolve** — *building this first.* The dye field is already a
full-screen scalar mask. Inject a wall of dye on a downbeat and threshold it to
blend between two sets of parameters. The transition is the fluid itself, so it
curls, and it is different every time.

**Vein blowout** — the veins flare white-hot over a couple of hundred
milliseconds, bloom saturates the frame, and the change happens inside the
flash. Cheap, and it leans on the one warm element the palette has. Wants a big
accent under it.

**Push through the blob** — the camera dollies into the surface and comes out
somewhere else. Diegetic. The raymarch already handles being inside a shell,
since the room is one; the work is in not clipping badly at the crossing.

**Shell collapse** — the room's radius shrinks past the camera, and by the time
the wall has gone by, the fbm parameters and palette have changed. Same shader,
different constants.

## Effects

**Ferrofluid spikes** — *building this first.* The blob grows spikes along its
normals, driven by band energy. Same material, same seams, unrecognisable
silhouette, and the most violently music-reactive thing available for the least
code.

**Vein tunnel** — fly down a tube whose walls are the same fbm and the same
molten veins, beads streaming past, the fluid layer reading as drag in the flow.
Reuses the room shader almost verbatim against a different SDF. The one that
would read as a genuinely second scene.

**Ink calligraphy** — beads leave persistent trails drawn into the dye field,
writing a shape or the group name, which then dissolves into the fluid. Ties
particles, fluid and text together, which makes it the strongest thread of the
lot.

**Shard mirror** — Voronoi-fracture the room shell into floating panels, each
catching a vein. Does not need the blob: the room's own walls and veins are
enough to reflect. The better version is that the shards *are* the blob, shed
when it shatters and gathered back when it reforms.

## Known gaps

- Nothing changes across the tune yet; the scene at 4:00 is the scene at 0:15.
- Depth of field: the depth buffer is already there, and focus pulls punctuate
  cuts well.
- Particle trails: cheap, since bead positions are analytic — draw each one
  several times at `t - k*dt`.
- `LATENCY_OFFSET_MS` is set by eye and probably wants another pass.
- No music is committed, and shipping one means clearing it with its author.
