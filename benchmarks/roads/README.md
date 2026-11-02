# Road extraction: validation

No field data or ground truth are available, so the method is checked at four levels, from
strongest to weakest. Every result is reported with its level, so agreement is never presented
as accuracy.

| Level | What | Status |
|---|---|---|
| 1 | Synthetic scenes with known geometry (winding road + straight road that cross, ditches, overhanging crowns, slopes 0–12 %, 1 and 5 pts/m², vertical noise 0.05 and 0.3 m, widths 3, 5, 8 m) | done: `baseline.md`, `baseline.json` |
| 2 | Stability: half and a fifth of the points, shuffled order, shifted origin, two tiles with overlap | done (same files) |
| 3 | Agreement with public references (official road network, OpenStreetMap) in a real area | pending: needs a real classified cloud and the reference layer for it; report as agreement, not accuracy |
| 4 | Visual review of a sample of outputs, with screenshots | pending: checklist below |

## Reproduce

    cargo run --release -p hipso-core --example roads_bench -- benchmarks/roads

The generator (`crates/core/examples/roads_bench.rs`) uses a fixed seed (20261102): every run
gives the same clouds and the same numbers. Metrics: centre line deviation (mean and Hausdorff,
m), width error (m), length matched within 5 m (recall) and share of the extracted length within
5 m of the truth (precision), road segments missed and invented, and number of lines.

## Thresholds

**None are set.** They are to be fixed with the user from `baseline.md`, not before. The
baseline shows where the method is weak, for example at 1 pt/m² with 0.3 m of noise, and that it
returns many short lines (the skeleton breaks at every junction of its ragged edge).

## Limits of the synthetic scenes

Bare fields, quarries, dry river beds and paths of other kinds are not modelled: in them, bare
ground is not a road, and the ground-share criterion cannot tell them apart. The surface of the
road is modelled only by its intensity (120 against 40 for the bare ground around it), and
`RoadParams::intensity` is off by default.

## Visual checklist (level 4)

For each sampled area, with the lines over the hillshade and the point cloud:

- [ ] Every road visible in the cloud has a line; none is cut at a crown or a car.
- [ ] Lines follow the centre of the road, not an edge or a ditch.
- [ ] Junctions have one meeting point, not a bundle of short lines.
- [ ] No line on a field, a firebreak, a dry river bed or a quarry floor.
- [ ] The width attribute agrees with the width seen (measure three places).
- [ ] Short side spurs that are not roads are absent.
- [ ] Nothing changes at tile borders.
