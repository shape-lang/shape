# ADR-011–016 tracker publication

Status: **published and independently audited**. The frozen plan completed at
`2026-07-27T12:18:38.027Z`; the independent live audit passed with zero
blockers, majors, or minors.

The machine-readable state, including all 67 symbolic issue mappings, is in
[`tracker-publication.json`](./tracker-publication.json).

## Authority boundary

[`program-manifest.draft.json`](./program-manifest.draft.json) is immutable
prepublication input. Its `claims.tracker_published: false` value must remain
unchanged: it records the state in which the publication was approved, not the
current tracker state.

This closeout is the postpublication state authority until
[AUTHORITY-BASELINE #111](https://github.com/shape-lang/shape/issues/111) lands
the committed baseline.

## Published state

- 89 publish-now entries: 67 new issues (`#111`–`#177`), 21 amended issues, and
  one byte-preserved issue (`#93`).
- 285 exact native blocker edges after 15 authorized obsolete-edge removals.
- 90 targets carry `ready-for-agent`; native edges, not that label, determine
  the runnable frontier.
- The sole ready zero-blocker frontier is
  [AUTHORITY-BASELINE #111](https://github.com/shape-lang/shape/issues/111).
- The publication journal is complete at 571/571 operations with no error.
- Seven expansion templates remain unmaterialized and have no issue mapping.

Special dispositions:

- `#22` received blocker-section-only surgery, has zero incoming blockers,
  remains unready, and still blocks `#23`.
- `#58` received blocker-section-only surgery, remains ready, and is blocked
  exactly by `MIGRATION-GUARD` (`#136`).
- `#93` retained its exact title and body; its body SHA-256 is
  `10470a7effbb018e9b38e6041c3abe35f47954fb838cb7acb32e3b198a899dd8`.

## Handoff

Claim [AUTHORITY-BASELINE #111](https://github.com/shape-lang/shape/issues/111)
first. Do not materialize any `*-WAVE-*` template until its owning inventory
has satisfied the manifest's atomic expansion protocol.

## Frozen evidence

| Artifact | SHA-256 |
|---|---|
| Draft manifest | `b1819b87506cf785d603f6b4b3c9a5a06f5a57ab71a02f41ceae07d2310d0591` |
| Logical publication plan | `1e166673b17d512d38056918d74fd2516da54d7cb490a83cfbb32ae14970144e` |
| Publication plan file | `1ba196dffc2810a97ea14063173348195e8c834692965fa73bdc3e7c9c68f91f` |
| Final mapping | `0ce8eada7429362d915715b3ebc3fea6ffb33358b15c3310d144d3d2c83433c6` |
| Completed journal | `19296fb6d4e200eb7c194e491b018c19b805885e6e8a30684eaf0032ffcbebdb` |
| Publisher prelabel snapshot | `ab2cf3eb7496c5d59d68d7be75227c7aeb806701e472fc9ca5b34ebf577d9c65` |
| Publisher final snapshot | `51a8bc5ab1c3e1fa6587ce4484953c236acd99b9ea9cd35ec6929ace587abe24` |
| Independent final snapshot | `97f208f5f55b643bf1628d9f9846f3fa9cab0d8fbcb49b0426ca7c87026aa93b` |

The independent audit used GitHub REST reads only and left 2,412 of 5,000 core
requests and 5,000 of 5,000 GraphQL requests available. No Cargo, build, or
test command was run during publication or audit.
