#!/usr/bin/env node
//
// ADR-016 §3 / §5 / §10 / R19 — the BookCoverageManifest contract gate
// (#113, BOOK-CONTRACT). Sibling of check-adr011-012-public-feature-manifest.mjs
// (#112), which gates the other half of the pair.
//
// Exit 0 = the coverage manifest is a legal successor of the previous accepted
// coverage manifest. Exit 1 = a contract violation. Exit 2 = unusable inputs.
//
// The PublicFeatureManifest says what must be explained; this manifest says
// where the Book explains it. The rules below exist to stop the coverage half
// of the same failure #112 defends against — making a gate green by editing the
// denominator rather than by writing documentation:
//
//   * a fence's classification fixes its obligations, so a hard example cannot
//     be quietly relabelled illustrative-only to stop being executed, and an
//     illustrative fence cannot carry an expectation it never has to meet;
//   * section and fence identities never contain a line number or an ordinal
//     position, so moving prose cannot read as removing and adding a feature;
//   * every identity ever published stays reachable — live, or tombstoned with
//     a reason and a citation — so coverage can never silently shrink;
//   * a fence identity is declared exactly once, because a fence is one
//     physical block, while a section may legitimately serve several features
//     provided every declaration agrees; and
//   * the manifest carries no source revision, counterpart SHA, attestation or
//     mutable verification state, because ADR-016 §7 keeps those in the
//     external pair evidence and a source manifest that stored them would make
//     the two repositories self-referential.
//
// The last rule is enforced against the schema as well as the manifest: a
// schema edit that DECLARES such a property fails here, before any manifest can
// carry one.
//
// With --public-features, the bidirectional join of ADR-016 §3 also runs: every
// non-removed public feature has an entry, every entry names a real feature,
// and every required mode, evidence class and semantic dimension has an owning
// section or fence. That join is the BookTruthGate's acceptance rule (§6), and
// running it against the committed pair today reports real, unfixed Book gaps —
// see docs/program/adr011-012/book-coverage-manifest.md §7.
//
// `--self-test` runs the forced negatives. Every tripwire asserts both that the
// unmutated input is accepted and that the mutation is rejected, so a gate that
// rejects everything cannot pass as a gate that works.

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { validateJsonSchema202012 } from "./lib/adr011-012-json-schema.mjs";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDirectory, "..");
const programDirectory = path.join(repositoryRoot, "docs/program/adr011-012");
const defaultManifestPath = path.join(programDirectory, "book-coverage-manifest.json");
const defaultSchemaPath = path.join(programDirectory, "book-coverage-manifest.schema.json");
const publicFeatureSchemaPath = path.join(programDirectory, "public-feature-manifest.schema.json");

const EXPECTED_SCHEMA_ID = "https://shape-lang.org/schemas/book-coverage-manifest.v1.schema.json";
const EXPECTED_SCHEMA_MAJOR = 1;
const EXPECTED_PUBLIC_FEATURE_MAJOR = 1;

// The three enums this manifest shares with the PublicFeatureManifest. They are
// obligations declared on one side and discharged on the other, so a member
// present in one schema and absent from the other is an obligation that can be
// declared and never satisfied, or satisfied and never declared.
const SHARED_ENUMS = ["mode", "evidenceClass", "semanticDimension"];

// ADR-016 §4, in table order. A feature whose public row sets
// distributed_semantics_required must map every one of them.
const DISTRIBUTED_DIMENSIONS = [
  "invocation", "effects-and-permissions", "provider-lifecycle", "discovery-and-admission",
  "execution-certainty", "ownership-transfer", "retry", "time-and-cancellation",
  "recovery", "cleanup", "persistence", "compatibility", "security", "observability",
  "operations", "degraded-modes",
];

// ADR-016 §3 / §7: exact revisions, counterpart hashes, attestation digests and
// mutable "last verified" state belong to the external PairCandidate, adapter
// reports and PairAttestation — never to a source manifest.
const FORBIDDEN_KEY_TOKENS = new Set([
  "sha", "sha1", "sha256", "sha512", "digest", "hash", "checksum", "oid",
  "revision", "rev", "commit", "sha256sum",
  "attest", "attested", "attestation", "attestations",
  "signature", "signatures", "signed",
  "verified", "verification", "verify", "verifying",
  "report", "reports", "candidate", "counterpart", "pair",
  "promotion", "promoted", "generation", "timestamp", "timestamps", "checked",
]);

// The single legal exception: the previous accepted manifest's own content
// identity, which ADR-016 §3 requires so the gate can compare against it.
const PRIOR_IDENTITY_PATH = "identity_migration/previous_manifest/sha256";

const BARE_DIGEST = /(?:^|[^0-9a-fA-F])([0-9a-fA-F]{40}|[0-9a-fA-F]{64})(?:[^0-9a-fA-F]|$)/;

function isObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function sameSet(left, right) {
  const a = new Set(left);
  const b = new Set(right);
  return a.size === b.size && [...a].every((value) => b.has(value));
}

function forbiddenTokenIn(key) {
  return key
    .split(/[_\-.]/)
    .map((token) => token.toLowerCase())
    .find((token) => FORBIDDEN_KEY_TOKENS.has(token));
}

function readJson(filePath, label) {
  try {
    return JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch (error) {
    console.error(`FATAL  #113: cannot read ${label} ${filePath}: ${error.message}`);
    process.exit(2);
  }
}

// Content identity is canonical-JSON based, not file-bytes based, so that
// reformatting the manifest does not read as a different prior identity.
function canonicalJson(value) {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (isObject(value)) {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
}

function contentIdentity(manifest) {
  // The migration record names the PREVIOUS manifest, so it is excluded from the
  // identity it would otherwise have to contain.
  const { identity_migration: _migration, $schema: _schema, ...rest } = manifest;
  return crypto.createHash("sha256").update(canonicalJson(rest)).digest("hex");
}

// A fence's classification determines its whole shape, exactly as a feature's
// status_basis determines its status in #112. Recovering the classification from
// the fields present is what makes "this fence is illustrative" a claim the gate
// can contradict rather than a label it must accept.
function deriveClassification(fence) {
  const gatedFields = ["evidence_role", "declared_modes", "expectation"].filter((name) => name in fence);
  const illustrativeField = "illustrative" in fence;
  if (gatedFields.length === 3 && !illustrativeField) return "runnable-gated";
  if (gatedFields.length === 0 && illustrativeField) return "illustrative-only";
  return undefined;
}

// --- schema guard --------------------------------------------------------

function checkSchema(schema, publicFeatureSchema, push) {
  if (!isObject(schema)) return push("schema must be an object");
  if (schema.$schema !== "https://json-schema.org/draft/2020-12/schema") {
    push("schema must use JSON Schema draft 2020-12");
  }
  if (schema.$id !== EXPECTED_SCHEMA_ID) push(`schema $id must be ${EXPECTED_SCHEMA_ID}`);

  const classificationEnum = schema.$defs?.fence?.properties?.classification?.enum;
  if (JSON.stringify(classificationEnum) !== JSON.stringify(["runnable-gated", "illustrative-only"])) {
    push(`schema fence classification enum must be exactly runnable-gated, illustrative-only, found ${JSON.stringify(classificationEnum)}`);
  }
  const distributedEnum = schema.$defs?.distributedDimension?.enum;
  if (JSON.stringify(distributedEnum) !== JSON.stringify(DISTRIBUTED_DIMENSIONS)) {
    push(`schema distributedDimension enum must be exactly the ADR-016 §4 matrix in table order, found ${JSON.stringify(distributedEnum)}`);
  }

  // The two manifests are one contract split across a repository boundary. An
  // obligation the public manifest can declare and this one cannot express is a
  // coverage rule that silently never applies, which is why the shared enums
  // must be identical rather than merely compatible.
  if (isObject(publicFeatureSchema)) {
    for (const name of SHARED_ENUMS) {
      const ours = schema.$defs?.[name]?.enum;
      const theirs = publicFeatureSchema.$defs?.[name]?.enum;
      if (JSON.stringify(ours) !== JSON.stringify(theirs)) {
        const missing = (theirs ?? []).filter((value) => !(ours ?? []).includes(value));
        const extra = (ours ?? []).filter((value) => !(theirs ?? []).includes(value));
        push(
          `schema $defs/${name} enum has drifted from the PublicFeatureManifest schema's` +
            (missing.length > 0 ? `; missing ${missing.join(", ")}` : "") +
            (extra.length > 0 ? `; unknown to the public manifest: ${extra.join(", ")}` : "") +
            (missing.length === 0 && extra.length === 0 ? " (same members, different order)" : ""),
        );
      }
    }
  }

  // Declared-property audit: the manifest cannot carry a revision, attestation
  // or mutable verification field if the schema cannot declare one.
  (function walk(node, pointer) {
    if (Array.isArray(node)) return node.forEach((child, index) => walk(child, `${pointer}/${index}`));
    if (!isObject(node)) return;
    for (const name of Object.keys(node.properties ?? {})) {
      const token = forbiddenTokenIn(name);
      const propertyPath = `${pointer}/properties/${name}`;
      // The one exemption is the prior accepted manifest's own content identity,
      // declared in exactly one place so it cannot be copied elsewhere.
      if (token && propertyPath !== "#/$defs/priorManifestIdentity/properties/sha256") {
        push(`schema declares forbidden property ${name} at ${propertyPath} (token "${token}"): ADR-016 §3 keeps exact revisions, attestation and mutable verification state out of source manifests`);
      }
    }
    // Every object node must close its shape, or an unknown field could enter
    // below the reach of the token audit above.
    if (node.type === "object" && !("additionalProperties" in node) && !("propertyNames" in node)) {
      push(`schema object at ${pointer} must set additionalProperties`);
    }
    for (const [key, child] of Object.entries(node)) {
      if (key === "properties" || key === "$defs") {
        for (const [name, grandchild] of Object.entries(child ?? {})) walk(grandchild, `${pointer}/${key}/${name}`);
      } else if (isObject(child) || Array.isArray(child)) {
        walk(child, `${pointer}/${key}`);
      }
    }
  })(schema, "#");
}

// --- single-manifest rules ----------------------------------------------

// Maps whose keys are author-chosen identifiers rather than field names. A
// section legitimately named `comptime.checked-shape` is not a `checked_at`
// verification field.
const IDENTIFIER_KEYED_MAPS = new Set([
  "features",
  "tombstones/feature_ids",
  "tombstones/section_ids",
  "tombstones/fence_ids",
  "tombstones/concept_ids",
  "identity_migration/feature_ids",
  "identity_migration/section_ids",
  "identity_migration/fence_ids",
]);

function isIdentifierKeyed(pointer) {
  if (IDENTIFIER_KEYED_MAPS.has(pointer)) return true;
  // features/<id>/{sections,fences,concept_identity_coverage} and the
  // essential_payload of a structured-diagnostic expectation are all keyed by
  // author-chosen names too.
  return /^features\/[^/]+\/(sections|fences|concept_identity_coverage)$/.test(pointer) ||
    /\/expectation\/essential_payload$/.test(pointer);
}

function checkRevisionFree(manifest, push) {
  (function walk(node, pointer) {
    if (Array.isArray(node)) return node.forEach((child, index) => walk(child, `${pointer}/${index}`));
    if (isObject(node)) {
      const keysAreIdentifiers = isIdentifierKeyed(pointer);
      for (const [key, child] of Object.entries(node)) {
        const childPointer = pointer ? `${pointer}/${key}` : key;
        const token = keysAreIdentifiers ? undefined : forbiddenTokenIn(key);
        if (token && childPointer !== PRIOR_IDENTITY_PATH) {
          push(`${childPointer}: forbidden field (token "${token}") — ADR-016 §3 keeps exact source/counterpart revisions, attestation digests and mutable verification state in the external pair evidence, not in this manifest`);
        }
        walk(child, childPointer);
      }
      return;
    }
    if (typeof node === "string" && pointer !== PRIOR_IDENTITY_PATH) {
      const digest = BARE_DIGEST.exec(node);
      if (digest) {
        push(`${pointer}: value embeds a bare ${digest[1].length}-character digest — a source or counterpart revision smuggled into a text field is still a revision pin (ADR-016 §7)`);
      }
    }
  })(manifest, "");
}

// Every stable identity the manifest uses, and where it came from. Built once so
// permanence, uniqueness and reference resolution all read the same view.
function indexIdentities(manifest) {
  const sections = new Map(); // section_id -> { featureIds: [], declarations: [] }
  const fences = new Map(); // fence_id -> { featureId, fence }
  for (const [featureId, coverage] of Object.entries(manifest.features ?? {})) {
    for (const [sectionId, section] of Object.entries(coverage.sections ?? {})) {
      if (!sections.has(sectionId)) sections.set(sectionId, { featureIds: [], declarations: [] });
      sections.get(sectionId).featureIds.push(featureId);
      sections.get(sectionId).declarations.push(section);
    }
    for (const [fenceId, fence] of Object.entries(coverage.fences ?? {})) {
      if (fences.has(fenceId)) {
        fences.get(fenceId).duplicateIn = featureId;
      } else {
        fences.set(fenceId, { featureId, fence });
      }
    }
  }
  return { sections, fences };
}

function checkCoverage(manifest, push) {
  const { sections, fences } = indexIdentities(manifest);
  const tombstones = manifest.tombstones ?? {};

  // A fence is one physical block, so its identity may be declared once. A
  // section may serve several features (ADR-016 §4), but every declaration must
  // agree — two different pages under one identity is the identity being reused.
  for (const [fenceId, record] of fences) {
    if (record.duplicateIn) {
      push(`fence ${fenceId}: declared by both ${record.featureId} and ${record.duplicateIn} — a fence is one physical block, so its identity is declared exactly once and referenced from elsewhere`);
    }
  }
  for (const [sectionId, record] of sections) {
    const [first, ...rest] = record.declarations;
    const disagreeing = rest.findIndex((declaration) => canonicalJson(declaration) !== canonicalJson(first));
    if (disagreeing >= 0) {
      push(`section ${sectionId}: declared differently by ${record.featureIds[0]} (${first.page}#${first.anchor}) and ${record.featureIds[disagreeing + 1]} (${rest[disagreeing].page}#${rest[disagreeing].anchor}) — one section identity is one place in the Book`);
    }
  }

  // A live identity that is also tombstoned is a retired identity reattached to
  // new material, which is ADR-016 §3 identity reuse.
  for (const sectionId of sections.keys()) {
    if (sectionId in (tombstones.section_ids ?? {})) {
      push(`section ${sectionId}: is live and also tombstoned — a retired section identity is never reused for new material (ADR-016 §3)`);
    }
  }
  for (const fenceId of fences.keys()) {
    if (fenceId in (tombstones.fence_ids ?? {})) {
      push(`fence ${fenceId}: is live and also tombstoned — a retired fence identity is never reused for new material (ADR-016 §3)`);
    }
  }
  for (const featureId of Object.keys(manifest.features ?? {})) {
    if (featureId in (tombstones.feature_ids ?? {})) {
      push(`${featureId}: has a live coverage entry and a coverage tombstone — stale Book coverage must not reattach to a retired identity (ADR-016 §9)`);
    }
  }

  for (const [featureId, coverage] of Object.entries(manifest.features ?? {})) {
    const localSections = new Set(Object.keys(coverage.sections ?? {}));
    const localFences = new Set(Object.keys(coverage.fences ?? {}));

    for (const [fenceId, fence] of Object.entries(coverage.fences ?? {})) {
      const derived = deriveClassification(fence);
      if (derived === undefined) {
        push(`${featureId}/${fenceId}: fields do not derive a classification — a runnable-gated fence carries evidence_role, declared_modes and expectation; an illustrative-only fence carries only illustrative`);
      } else if (derived !== fence.classification) {
        push(`${featureId}/${fenceId}: classification is "${fence.classification}" but its fields derive "${derived}" — ADR-016 §5 makes classification the fence's obligation, not a label beside it`);
      }
      if (!localSections.has(fence.section_id)) {
        push(`${featureId}/${fenceId}: section_id ${fence.section_id} is not declared by this feature's sections`);
      }
      // §10: a budget bounds a documented experience, so it only means anything
      // on a fence the gate actually executes; and only flagship fences carry one.
      if ("budget" in fence && !(manifest.flagship_fence_ids ?? []).includes(fenceId)) {
        push(`${featureId}/${fenceId}: declares a ceremony budget but is not in flagship_fence_ids — ADR-016 §10 attaches budgets to the enumerated flagship set`);
      }
    }

    // §10: the flagship set is ratcheted, and every member owes a budget.
    for (const fenceId of manifest.flagship_fence_ids ?? []) {
      const record = fences.get(fenceId);
      if (!record) continue;
      if (record.fence.classification !== "runnable-gated") {
        push(`flagship fence ${fenceId}: is ${record.fence.classification} — a ceremony budget is an acceptance gate on executed documentation`);
      }
      if (!("budget" in record.fence)) {
        push(`flagship fence ${fenceId}: declares no ceremony budget — ADR-016 §10 requires the designated flagship set to declare budgets`);
      }
    }

    // Every reference in the coverage maps must resolve to something declared.
    const maps = {
      mode_coverage: coverage.mode_coverage,
      semantic_dimension_coverage: coverage.semantic_dimension_coverage,
      evidence_class_coverage: coverage.evidence_class_coverage,
      distributed_dimension_coverage: coverage.distributed_dimension_coverage,
      concept_identity_coverage: coverage.concept_identity_coverage,
    };
    for (const [mapName, map] of Object.entries(maps)) {
      for (const [key, references] of Object.entries(map ?? {})) {
        for (const sectionId of references.section_ids ?? []) {
          if (!localSections.has(sectionId)) {
            push(`${featureId}/${mapName}/${key}: references section ${sectionId}, which this feature does not declare`);
          }
        }
        for (const fenceId of references.fence_ids ?? []) {
          if (!localFences.has(fenceId)) {
            push(`${featureId}/${mapName}/${key}: references fence ${fenceId}, which this feature does not declare`);
          }
        }
      }
    }

    // §5: parity is two real executions, so a fence claiming both modes is the
    // only thing that can discharge vm-jit-parity.
    for (const fenceId of coverage.evidence_class_coverage?.["vm-jit-parity"]?.fence_ids ?? []) {
      const fence = coverage.fences?.[fenceId];
      if (!fence) continue;
      const modes = new Set(fence.declared_modes ?? []);
      if (!modes.has("vm") || !modes.has("jit")) {
        push(`${featureId}/${fenceId}: is cited for vm-jit-parity but declares modes ${[...modes].join(", ") || "none"} — ADR-016 §5 makes parity two real executions, so the fence must declare both vm and jit`);
      }
    }

    // §10: every concept identity a gated negative fence asserts must be taught
    // somewhere. An unmapped concept identity in gated evidence is a failure.
    const mappedConcepts = new Set(Object.keys(coverage.concept_identity_coverage ?? {}));
    for (const [fenceId, fence] of Object.entries(coverage.fences ?? {})) {
      const conceptId = fence.expectation?.concept_id;
      if (conceptId && !mappedConcepts.has(conceptId)) {
        push(`${featureId}/${fenceId}: asserts diagnostic concept ${conceptId} with no section that teaches it — ADR-016 §10 makes an unmapped concept identity in gated evidence a coverage failure`);
      }
    }
  }

  // Tombstone integrity, in every identity domain.
  const tombstoneDomains = [
    ["feature_ids", tombstones.feature_ids, "replacement_feature_ids", new Set(Object.keys(manifest.features ?? {}))],
    ["section_ids", tombstones.section_ids, "replacement_ids", new Set(sections.keys())],
    ["fence_ids", tombstones.fence_ids, "replacement_ids", new Set(fences.keys())],
  ];
  for (const [domain, table, replacementField, live] of tombstoneDomains) {
    for (const [id, tombstone] of Object.entries(table ?? {})) {
      for (const replacement of tombstone[replacementField] ?? []) {
        if (replacement === id) {
          push(`tombstones/${domain}/${id}: names itself as its replacement`);
        } else if (!live.has(replacement)) {
          push(`tombstones/${domain}/${id}: replacement ${replacement} is not a live identity — a tombstone must name real successor material (ADR-016 §3)`);
        }
      }
    }
  }
}

// --- the bidirectional join against the PublicFeatureManifest ------------

function checkAgainstPublicFeatures(manifest, publicFeatures, push) {
  const features = publicFeatures.features ?? {};
  const coverage = manifest.features ?? {};

  if (manifest.public_feature_manifest?.schema_major !== publicFeatures.manifest_version) {
    push(`public_feature_manifest.schema_major is ${manifest.public_feature_manifest?.schema_major} but the supplied PublicFeatureManifest is version ${publicFeatures.manifest_version}`);
  }

  for (const [featureId, feature] of Object.entries(features)) {
    const entry = coverage[featureId];
    if (feature.status === "removed") {
      if (entry) {
        push(`${featureId}: is removed in the PublicFeatureManifest but still has a live coverage entry — a removed feature keeps a tombstone, so stale Book coverage cannot reattach (ADR-016 §9)`);
      }
      continue;
    }
    if (!entry) {
      push(`${featureId}: is ${feature.status} in the PublicFeatureManifest with no coverage entry — ADR-016 §3 makes a non-removed public feature without coverage a gate failure`);
      continue;
    }

    const covered = (map) => new Set(Object.keys(map ?? {}));
    const missing = (required, map, label) => {
      const have = covered(map);
      const gaps = (required ?? []).filter((value) => !have.has(value));
      if (gaps.length > 0) {
        push(`${featureId}: required ${label} with no owning section or fence: ${gaps.join(", ")} — the coverage manifest may not weaken a feature's declared dimensions (ADR-016 §3)`);
      }
    };
    missing(feature.required_modes, entry.mode_coverage, "mode(s)");
    missing(feature.required_evidence_classes, entry.evidence_class_coverage, "evidence class(es)");
    missing(feature.required_semantic_dimensions, entry.semantic_dimension_coverage, "semantic dimension(s)");
    if (feature.distributed_semantics_required) {
      missing(DISTRIBUTED_DIMENSIONS, entry.distributed_dimension_coverage, "distributed dimension(s)");
    }

    // §2: a planned feature is not presented as current. Its Book coverage is
    // limited to planned/illustrative material or a structured rejection; it
    // cannot carry successful execution evidence as if it were available.
    if (feature.status === "planned") {
      for (const [fenceId, fence] of Object.entries(entry.fences ?? {})) {
        if (fence.classification !== "runnable-gated") continue;
        if (!["negative", "diagnostic"].includes(fence.evidence_role)) {
          push(`${featureId}/${fenceId}: the feature is planned but this gated fence claims evidence role "${fence.evidence_role}" — ADR-016 §2 limits planned coverage to planned/illustrative material or a runnable structured rejection`);
        }
      }
    }

    // §10: the script-tier dimension is discharged by a mechanically checked
    // zero-ceremony fence, not by prose about how little ceremony there is.
    if ((feature.required_semantic_dimensions ?? []).includes("script-tier")) {
      const zeroCeremony = Object.entries(entry.fences ?? {}).filter(
        ([, fence]) => fence.classification === "runnable-gated" && fence.ceremony === "none",
      );
      if (zeroCeremony.length === 0) {
        push(`${featureId}: declares the script-tier dimension with no runnable-gated fence declaring ceremony "none" — ADR-016 §10 requires the mechanical zero-ceremony check, not a prose claim`);
      }
    }
  }

  for (const featureId of Object.keys(coverage)) {
    if (!(featureId in features)) {
      push(`${featureId}: has a coverage entry but is not in the PublicFeatureManifest — the coverage manifest may not invent a feature identity (ADR-016 §3)`);
    }
  }
}

// --- candidate-versus-previous rules ------------------------------------

function checkPair(manifest, previous, expectedMajor, push) {
  const migration = manifest.identity_migration ?? {};

  if (manifest.manifest_version !== expectedMajor) {
    push(`manifest_version must be ${expectedMajor}, found ${manifest.manifest_version}`);
  }
  if (manifest.public_feature_manifest?.schema_major !== EXPECTED_PUBLIC_FEATURE_MAJOR) {
    push(`public_feature_manifest.schema_major must be ${EXPECTED_PUBLIC_FEATURE_MAJOR}, found ${manifest.public_feature_manifest?.schema_major}`);
  }

  if (!previous) {
    if (migration.kind !== "initial") {
      push(`identity_migration.kind is "${migration.kind}" but no previous accepted manifest was supplied — pass --previous, or declare kind "initial"`);
    }
    return;
  }

  if (migration.kind === "initial") {
    return push("identity_migration.kind is \"initial\" but a previous accepted manifest was supplied — an initial manifest has no predecessor");
  }

  const identity = contentIdentity(previous);
  if (migration.previous_manifest?.sha256 !== identity) {
    push(`identity_migration.previous_manifest.sha256 is ${migration.previous_manifest?.sha256}, but the supplied previous manifest has content identity ${identity}`);
  }
  const expectedPriorMajor = migration.kind === "schema-major" ? manifest.manifest_version - 1 : manifest.manifest_version;
  if (migration.previous_manifest?.schema_major !== expectedPriorMajor) {
    push(`identity_migration.previous_manifest.schema_major must be ${expectedPriorMajor} for a ${migration.kind} migration, found ${migration.previous_manifest?.schema_major}`);
  }

  const before = indexIdentities(previous);
  const after = indexIdentities(manifest);
  const domains = [
    ["feature", new Set(Object.keys(previous.features ?? {})), new Set(Object.keys(manifest.features ?? {})), manifest.tombstones?.feature_ids, previous.tombstones?.feature_ids],
    ["section", new Set(before.sections.keys()), new Set(after.sections.keys()), manifest.tombstones?.section_ids, previous.tombstones?.section_ids],
    ["fence", new Set(before.fences.keys()), new Set(after.fences.keys()), manifest.tombstones?.fence_ids, previous.tombstones?.fence_ids],
  ];

  for (const [label, priorLive, currentLive, currentTombstones, priorTombstones] of domains) {
    // A published identity is permanent: it stays live, or it becomes a
    // tombstone with a reason. Disappearing is how a denominator shrinks.
    for (const id of priorLive) {
      if (currentLive.has(id)) continue;
      if (!(id in (currentTombstones ?? {}))) {
        push(`${label} ${id}: was live in the previous accepted manifest and is neither live nor tombstoned here — a published ${label} identity is permanent; retire it with a tombstone instead of dropping it (ADR-016 §3)`);
      }
    }
    // A tombstone is a record of what happened, so rewriting one reattaches a
    // retired identity to a new meaning.
    for (const [id, tombstone] of Object.entries(priorTombstones ?? {})) {
      const now = (currentTombstones ?? {})[id];
      if (!now) {
        push(`${label} ${id}: was tombstoned in the previous accepted manifest and its tombstone is gone here — a retired ${label} identity stays retired (ADR-016 §3)`);
      } else if (canonicalJson(now) !== canonicalJson(tombstone)) {
        push(`${label} ${id}: was tombstoned in the previous accepted manifest and its tombstone changed here — a retired ${label} identity is never reused for a different meaning (ADR-016 §3)`);
      }
    }
  }

  // A live section identity that now names a different place in the Book is the
  // identity being repurposed rather than the prose being edited.
  for (const [sectionId, record] of after.sections) {
    const priorRecord = before.sections.get(sectionId);
    if (!priorRecord) continue;
    const from = priorRecord.declarations[0];
    const to = record.declarations[0];
    if (from.page !== to.page || from.anchor !== to.anchor) {
      push(`section ${sectionId}: moved ${from.page}#${from.anchor} -> ${to.page}#${to.anchor} — a published section identity is never reused for different material; give the new place a new identity and tombstone this one (ADR-016 §3)`);
    }
  }

  // §5: the illustrative set is ratcheted. Downgrading an executed fence to
  // illustrative-only is how a hard example stops being run.
  for (const [fenceId, record] of after.fences) {
    const priorRecord = before.fences.get(fenceId);
    if (!priorRecord) continue;
    if (priorRecord.fence.classification === "runnable-gated" && record.fence.classification === "illustrative-only") {
      push(`fence ${fenceId}: was runnable-gated and is now illustrative-only — ADR-016 §5 ratchets the illustrative set, and an example is not illustrative merely because the implementation is broken`);
    }
  }

  // §10: the flagship set is ratcheted the same way, and budgets tighten only.
  for (const fenceId of previous.flagship_fence_ids ?? []) {
    if (!(manifest.flagship_fence_ids ?? []).includes(fenceId)) {
      push(`flagship fence ${fenceId}: was in the flagship set and is not here — ADR-016 §10 requires explicit review to remove a fence from the flagship set`);
      continue;
    }
    const from = before.fences.get(fenceId)?.fence?.budget;
    const to = after.fences.get(fenceId)?.fence?.budget;
    if (!from || !to) continue;
    for (const field of ["max_lines", "max_explicit_annotations"]) {
      if (to[field] > from[field]) {
        push(`flagship fence ${fenceId}: budget ${field} loosened ${from[field]} -> ${to[field]} — ADR-016 §10 makes tightening ordinary and loosening an explicitly reviewed change`);
      }
    }
  }

  if (migration.kind !== "schema-major") return;

  // §3: the migration map is total over the prior feature, section and fence
  // identity sets, so old coverage cannot silently attach to new material.
  const mapDomains = [
    ["feature_ids", migration.feature_ids, new Set([...Object.keys(previous.features ?? {}), ...Object.keys(previous.tombstones?.feature_ids ?? {})]), new Set(Object.keys(manifest.features ?? {})), manifest.tombstones?.feature_ids],
    ["section_ids", migration.section_ids, new Set([...before.sections.keys(), ...Object.keys(previous.tombstones?.section_ids ?? {})]), new Set(after.sections.keys()), manifest.tombstones?.section_ids],
    ["fence_ids", migration.fence_ids, new Set([...before.fences.keys(), ...Object.keys(previous.tombstones?.fence_ids ?? {})]), new Set(after.fences.keys()), manifest.tombstones?.fence_ids],
  ];

  for (const [domain, map, priorIds, currentLive, currentTombstones] of mapDomains) {
    if (!sameSet(Object.keys(map ?? {}), [...priorIds])) {
      const gaps = [...priorIds].filter((id) => !(id in (map ?? {})));
      const extra = Object.keys(map ?? {}).filter((id) => !priorIds.has(id));
      if (gaps.length > 0) {
        push(`identity_migration.${domain} is not total: ${gaps.join(", ")} ${gaps.length === 1 ? "exists" : "exist"} in the previous manifest with no disposition — a schema-major change carries a complete old-to-new identity migration map (ADR-016 §3)`);
      }
      if (extra.length > 0) {
        push(`identity_migration.${domain} names ${extra.join(", ")}, which are not in the previous manifest`);
      }
    }
    for (const [id, disposition] of Object.entries(map ?? {})) {
      const newIds = disposition.new_ids ?? [];
      if (disposition.action === "unchanged") {
        if (!sameSet(newIds, [id])) {
          push(`identity_migration.${domain}/${id}: an unchanged identity must map to itself, found ${newIds.join(", ") || "nothing"}`);
        }
      } else if (disposition.action === "replaced") {
        if (currentLive.has(id)) {
          push(`identity_migration.${domain}/${id}: replaced by ${newIds.join(", ")} but is still live here — the old identifier becomes a tombstone (ADR-016 §3)`);
        }
        if (!(id in (currentTombstones ?? {}))) {
          push(`identity_migration.${domain}/${id}: replaced by ${newIds.join(", ")} with no tombstone`);
        }
        for (const newId of newIds) {
          if (priorIds.has(newId)) {
            push(`identity_migration.${domain}/${id}: replacement ${newId} already existed in the previous manifest — a replacement is a new identity, not a reused one`);
          } else if (!currentLive.has(newId)) {
            push(`identity_migration.${domain}/${id}: replacement ${newId} is not live in this manifest`);
          }
        }
      } else if (disposition.action === "removed") {
        if (currentLive.has(id)) {
          push(`identity_migration.${domain}/${id}: migrated as removed but is still live here`);
        }
        if (!(id in (currentTombstones ?? {}))) {
          push(`identity_migration.${domain}/${id}: migrated as removed with no tombstone`);
        }
      }
    }
  }
}

// --- engine --------------------------------------------------------------

export function validateManifest({
  schema,
  publicFeatureSchema,
  manifest,
  previous,
  publicFeatures,
  expectedMajor = EXPECTED_SCHEMA_MAJOR,
}) {
  const errors = [];
  const push = (message) => errors.push(message);

  checkSchema(schema, publicFeatureSchema, push);
  for (const error of validateJsonSchema202012(schema, manifest).errors) push(`JSON Schema: ${error}`);
  if (previous) {
    for (const error of validateJsonSchema202012(schema, previous).errors) {
      push(`JSON Schema (previous accepted manifest): ${error}`);
    }
  }
  if (errors.length > 0) return errors;

  if (manifest.$schema !== "./book-coverage-manifest.schema.json") {
    push("manifest must reference the local schema as ./book-coverage-manifest.schema.json");
  }
  checkRevisionFree(manifest, push);
  checkCoverage(manifest, push);
  checkPair(manifest, previous, expectedMajor, push);
  if (publicFeatures) checkAgainstPublicFeatures(manifest, publicFeatures, push);
  return errors;
}

// --- forced negatives ----------------------------------------------------

// Each tripwire mutates a legal input and asserts rejection. `expect` names the
// substring the failure must mention, so a rejection for an unrelated reason
// does not count as the tripwire firing.
//
// The fixtures are synthetic so the identity, classification and pairing
// tripwires run against a fixed shape rather than whatever the real coverage
// happens to contain today.
function fixtureSection(overrides = {}) {
  return { page: "selftest/fixture.mdx", anchor: "selftest-fixture", title: "Self-test fixture", ...overrides };
}

function fixtureGatedFence(overrides = {}) {
  return {
    section_id: "selftest.fixture.section",
    classification: "runnable-gated",
    evidence_role: "success",
    declared_modes: ["vm", "jit"],
    expectation: { kind: "stdout", value: "ok\n" },
    ...overrides,
  };
}

// Two fences, because `fences` has minProperties 1: with a single fence, the
// tripwire that drops one would be rejected by the schema's arity rule rather
// than by the identity-permanence rule it is meant to prove.
function fixtureCoverage(overrides = {}) {
  return {
    sections: { "selftest.fixture.section": fixtureSection() },
    fences: {
      "selftest.fixture.fence": fixtureGatedFence(),
      "selftest.fixture.second-fence": fixtureGatedFence({ expectation: { kind: "exit-success" } }),
    },
    mode_coverage: { vm: { fence_ids: ["selftest.fixture.fence"] } },
    semantic_dimension_coverage: { "user-model": { section_ids: ["selftest.fixture.section"] } },
    evidence_class_coverage: { "positive-execution": { fence_ids: ["selftest.fixture.fence"] } },
    distributed_dimension_coverage: {},
    ...overrides,
  };
}

// A complete manifest holding only the synthetic feature. The join tripwires use
// it so their fixture PublicFeatureManifest is an exact counterpart: pairing the
// committed manifest with a synthetic feature list would fail for the unrelated
// reason that the real feature is missing from the fixture.
function fixtureManifest(mutate = () => {}) {
  const value = {
    $schema: "./book-coverage-manifest.schema.json",
    manifest_version: 1,
    public_feature_manifest: { schema_major: 1 },
    identity_migration: { kind: "initial" },
    features: { "selftest.fixture": fixtureCoverage() },
    flagship_fence_ids: [],
    tombstones: { feature_ids: {}, section_ids: {}, fence_ids: {}, concept_ids: {} },
  };
  mutate(value);
  return value;
}

// A minimal PublicFeatureManifest the fixture coverage satisfies exactly, so the
// join's tripwires can each remove one thing rather than fight unrelated gaps.
function fixturePublicFeatures(overrides = {}) {
  return {
    manifest_version: 1,
    identity_migration: { kind: "initial" },
    features: {
      "selftest.fixture": {
        public_name: "Self-test fixture",
        family: "self-test",
        status: "public",
        status_basis: { classification: "current-executable", evidence: [{ kind: "adr", reference: "ADR-016" }] },
        authority: [{ kind: "adr", reference: "ADR-016" }],
        owner: { repository: "shape-lang/shape", component: "self-test fixture" },
        surfaces: ["language-syntax"],
        targets: ["expression"],
        required_modes: ["vm"],
        required_evidence_classes: ["positive-execution"],
        required_semantic_dimensions: ["user-model"],
        distributed_semantics_required: false,
        ...overrides,
      },
    },
  };
}

function tripwires(schema, publicFeatureSchema, manifest) {
  const liveFeatureId = Object.keys(manifest.features)[0];
  const liveFenceId = Object.keys(manifest.features[liveFeatureId].fences)[0];
  const liveSectionId = Object.keys(manifest.features[liveFeatureId].sections)[0];

  // The committed manifest plus one synthetic feature whose coverage is
  // self-contained. Used wherever a tripwire needs a fixed shape.
  const withFixture = (mutate = () => {}) => {
    const value = structuredClone(manifest);
    value.features["selftest.fixture"] = fixtureCoverage();
    mutate(value);
    return value;
  };

  // A legal successor: same rows, declared as a revision naming the predecessor.
  const successorOf = (previous, mutate = () => {}) => {
    const candidate = structuredClone(previous);
    candidate.identity_migration = {
      kind: "revision",
      previous_manifest: { schema_major: previous.manifest_version, sha256: contentIdentity(previous) },
    };
    mutate(candidate);
    return candidate;
  };

  const citation = { kind: "adr", reference: "ADR-016" };

  return [
    {
      id: "T1a live fence identity dropped instead of tombstoned",
      previous: withFixture(),
      candidate: successorOf(withFixture(), (value) => {
        delete value.features["selftest.fixture"].fences["selftest.fixture.fence"];
        delete value.features["selftest.fixture"].mode_coverage.vm;
        delete value.features["selftest.fixture"].evidence_class_coverage["positive-execution"];
      }),
      expect: "a published fence identity is permanent",
    },
    {
      id: "T1b live feature coverage dropped instead of tombstoned",
      previous: withFixture(),
      candidate: successorOf(withFixture(), (value) => {
        delete value.features["selftest.fixture"];
      }),
      expect: "a published feature identity is permanent",
    },
    {
      id: "T2a retired identity reattached — tombstone rewritten",
      previous: withFixture((value) => {
        value.tombstones.fence_ids["selftest.retired.fence"] = { reason: "self-test fixture", citation };
      }),
      candidate: successorOf(
        withFixture((value) => {
          value.tombstones.fence_ids["selftest.retired.fence"] = { reason: "self-test fixture", citation };
        }),
        (value) => {
          value.tombstones.fence_ids["selftest.retired.fence"].reason = "a completely different removal";
        },
      ),
      expect: "never reused for a different meaning",
    },
    {
      id: "T2b live section identity repointed to different material",
      previous: withFixture(),
      candidate: successorOf(withFixture(), (value) => {
        value.features["selftest.fixture"].sections["selftest.fixture.section"].page = "selftest/somewhere-else.mdx";
      }),
      expect: "never reused for different material",
    },
    {
      id: "T2c one fence identity declared by two features",
      previous: undefined,
      candidate: withFixture((value) => {
        value.features["selftest.fixture"].fences[liveFenceId] = fixtureGatedFence();
      }),
      expect: "a fence is one physical block",
    },
    {
      id: "T3a manifest carrying a source SHA",
      previous: undefined,
      candidate: { ...structuredClone(manifest), source_sha: "0".repeat(40) },
      expect: "additional property is forbidden",
    },
    {
      id: "T3b digest smuggled into a text field",
      previous: undefined,
      candidate: (() => {
        const value = structuredClone(manifest);
        value.features[liveFeatureId].sections[liveSectionId].title = `verified at ${"a1b2c3d4".repeat(8)}`;
        return value;
      })(),
      expect: "bare 64-character digest",
    },
    {
      id: "T3c schema declaring an attestation field",
      schema: (() => {
        const value = structuredClone(schema);
        value.$defs.fence.properties.attestation_digest = { type: "string" };
        return value;
      })(),
      previous: undefined,
      candidate: manifest,
      expect: "schema declares forbidden property attestation_digest",
    },
    // T4: a fence's classification is a function of the fields it carries.
    // Proven by substitution: leave the fields alone and the other
    // classification is rejected, so the declared one is the only one its
    // obligations admit.
    {
      id: "T4a runnable-gated fence relabelled illustrative-only",
      previous: undefined,
      candidate: (() => {
        const value = structuredClone(manifest);
        value.features[liveFeatureId].fences[liveFenceId].classification = "illustrative-only";
        return value;
      })(),
      expect: liveFenceId,
    },
    {
      id: "T4b illustrative-only fence relabelled runnable-gated",
      previous: undefined,
      candidate: withFixture((value) => {
        value.features["selftest.fixture"].fences["selftest.fixture.illustration"] = {
          section_id: "selftest.fixture.section",
          classification: "runnable-gated",
          illustrative: { reason: "self-test fixture", citation },
        };
      }),
      expect: "selftest.fixture.illustration",
    },
    {
      id: "T4c executed fence downgraded to illustrative-only",
      previous: withFixture(),
      candidate: successorOf(withFixture(), (value) => {
        const fence = value.features["selftest.fixture"].fences["selftest.fixture.fence"];
        delete fence.evidence_role;
        delete fence.declared_modes;
        delete fence.expectation;
        fence.classification = "illustrative-only";
        fence.illustrative = { reason: "self-test fixture", citation };
        delete value.features["selftest.fixture"].mode_coverage.vm;
        delete value.features["selftest.fixture"].evidence_class_coverage["positive-execution"];
      }),
      expect: "ratchets the illustrative set",
    },
    {
      id: "T5a schema-major migration map is not total",
      previous: withFixture(),
      candidate: (() => {
        const previous = withFixture();
        const value = structuredClone(previous);
        value.manifest_version = previous.manifest_version + 1;
        value.identity_migration = {
          kind: "schema-major",
          previous_manifest: { schema_major: previous.manifest_version, sha256: contentIdentity(previous) },
          feature_ids: { [liveFeatureId]: { action: "unchanged", new_ids: [liveFeatureId], citation } },
          section_ids: { [liveSectionId]: { action: "unchanged", new_ids: [liveSectionId], citation } },
          fence_ids: { [liveFenceId]: { action: "unchanged", new_ids: [liveFenceId], citation } },
        };
        return value;
      })(),
      expectedMajor: manifest.manifest_version + 1,
      expect: "is not total",
    },
    {
      id: "T5b tombstone dropped instead of retained",
      previous: withFixture((value) => {
        value.tombstones.section_ids["selftest.retired.section"] = { reason: "self-test fixture", citation };
      }),
      candidate: successorOf(
        withFixture((value) => {
          value.tombstones.section_ids["selftest.retired.section"] = { reason: "self-test fixture", citation };
        }),
        (value) => {
          delete value.tombstones.section_ids["selftest.retired.section"];
        },
      ),
      expect: "stays retired",
    },
    {
      id: "T6 dangling coverage reference",
      previous: undefined,
      candidate: withFixture((value) => {
        value.features["selftest.fixture"].semantic_dimension_coverage["user-model"] = {
          section_ids: ["selftest.fixture.nowhere"],
        };
      }),
      expect: "which this feature does not declare",
    },
    {
      id: "T7 shared enum drifted from the PublicFeatureManifest schema",
      schema: (() => {
        const value = structuredClone(schema);
        value.$defs.semanticDimension.enum = value.$defs.semanticDimension.enum.filter(
          (name) => name !== "script-tier",
        );
        return value;
      })(),
      previous: undefined,
      candidate: manifest,
      expect: "has drifted from the PublicFeatureManifest schema",
    },
    {
      id: "T8a public feature with no coverage entry",
      previous: undefined,
      candidate: fixtureManifest((value) => {
        delete value.features["selftest.fixture"];
        value.features["selftest.other"] = fixtureCoverage();
      }),
      publicFeatures: (() => {
        const value = fixturePublicFeatures();
        value.features["selftest.other"] = structuredClone(value.features["selftest.fixture"]);
        value.features["selftest.other"].public_name = "Self-test fixture (other)";
        return value;
      })(),
      controlCandidate: fixtureManifest(),
      controlPublicFeatures: fixturePublicFeatures(),
      expect: "with no coverage entry",
    },
    {
      id: "T8b coverage entry inventing a feature identity",
      previous: undefined,
      candidate: fixtureManifest((value) => {
        value.features["selftest.invented"] = fixtureCoverage();
      }),
      publicFeatures: fixturePublicFeatures(),
      controlCandidate: fixtureManifest(),
      controlPublicFeatures: fixturePublicFeatures(),
      expect: "may not invent a feature identity",
    },
    {
      id: "T8c required semantic dimension with no owning section or fence",
      previous: undefined,
      candidate: fixtureManifest(),
      publicFeatures: fixturePublicFeatures({
        required_semantic_dimensions: ["user-model", "failure"],
      }),
      controlCandidate: fixtureManifest(),
      controlPublicFeatures: fixturePublicFeatures(),
      expect: "required semantic dimension(s) with no owning section or fence: failure",
    },
    {
      id: "T8d planned feature carrying successful execution evidence",
      previous: undefined,
      candidate: fixtureManifest(),
      publicFeatures: fixturePublicFeatures({
        status: "planned",
        status_basis: { classification: "never-current", evidence: [{ kind: "adr", reference: "ADR-016" }] },
        required_modes: [],
        required_evidence_classes: [],
      }),
      controlCandidate: fixtureManifest(),
      controlPublicFeatures: fixturePublicFeatures(),
      expect: "limits planned coverage",
    },
    {
      id: "T9 script-tier dimension without a ceremony:none fence",
      previous: undefined,
      candidate: fixtureManifest(),
      publicFeatures: fixturePublicFeatures({
        required_semantic_dimensions: ["user-model", "script-tier"],
        required_evidence_classes: ["positive-execution", "ceremony-budget"],
      }),
      controlCandidate: fixtureManifest(),
      controlPublicFeatures: fixturePublicFeatures(),
      // The dimension gap fires too; this asserts the mechanical-check rule
      // specifically, which is the one §10 adds.
      expect: "requires the mechanical zero-ceremony check",
    },
    {
      id: "T10 flagship fence with no ceremony budget",
      previous: undefined,
      candidate: withFixture((value) => {
        value.flagship_fence_ids = ["selftest.fixture.fence"];
      }),
      expect: "declares no ceremony budget",
    },
  ];
}

function runSelfTest(schema, publicFeatureSchema, manifest) {
  const failures = [];
  for (const tripwire of tripwires(schema, publicFeatureSchema, manifest)) {
    const activeSchema = tripwire.schema ?? schema;
    const expectedMajor = tripwire.expectedMajor ?? EXPECTED_SCHEMA_MAJOR;

    // Positive control: the same inputs without the mutation must be accepted,
    // so a gate that rejects everything cannot pass its own tripwires.
    const controlManifest =
      tripwire.controlCandidate ??
      (tripwire.previous
        ? (() => {
            const value = structuredClone(tripwire.previous);
            value.identity_migration = {
              kind: "revision",
              previous_manifest: { schema_major: value.manifest_version, sha256: contentIdentity(tripwire.previous) },
            };
            return value;
          })()
        : manifest);
    const controlErrors = validateManifest({
      schema,
      publicFeatureSchema,
      manifest: controlManifest,
      previous: tripwire.previous,
      publicFeatures: tripwire.controlPublicFeatures,
      expectedMajor: EXPECTED_SCHEMA_MAJOR,
    });
    if (controlErrors.length > 0) {
      failures.push(`${tripwire.id}: positive control was rejected — ${controlErrors[0]}`);
    }

    const errors = validateManifest({
      schema: activeSchema,
      publicFeatureSchema,
      manifest: tripwire.candidate,
      previous: tripwire.previous,
      publicFeatures: tripwire.publicFeatures,
      expectedMajor,
    });
    if (errors.length === 0) {
      failures.push(`${tripwire.id}: forced negative was ACCEPTED`);
    } else if (!errors.some((error) => error.includes(tripwire.expect))) {
      failures.push(`${tripwire.id}: rejected, but for the wrong reason — expected a failure mentioning "${tripwire.expect}", got: ${errors[0]}`);
    } else {
      console.log(`  ${tripwire.id}: rejected — ${errors.find((error) => error.includes(tripwire.expect))}`);
    }
  }
  return failures;
}

// --- cli -----------------------------------------------------------------

function parseArguments(argv) {
  const options = {
    manifestPath: defaultManifestPath,
    schemaPath: defaultSchemaPath,
    selfTest: false,
    printIdentity: false,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--help") {
      console.log("Usage: node scripts/check-adr011-012-book-coverage-manifest.mjs [--manifest path] [--schema path] [--previous path] [--public-features path] [--self-test] [--print-identity]");
      console.log("");
      console.log("  --previous        the previous ACCEPTED coverage manifest to validate against.");
      console.log("                    Omit only while identity_migration.kind is \"initial\".");
      console.log("  --public-features run the ADR-016 §3 bidirectional join against a");
      console.log("                    PublicFeatureManifest. This is BookTruthGate's acceptance");
      console.log("                    rule; against the committed pair it reports the Book's real");
      console.log("                    coverage gaps and exits 1.");
      console.log("  --print-identity  print this manifest's content identity, for the next");
      console.log("                    revision's identity_migration.previous_manifest.sha256.");
      process.exit(0);
    }
    if (argument === "--self-test") {
      options.selfTest = true;
      continue;
    }
    if (argument === "--print-identity") {
      options.printIdentity = true;
      continue;
    }
    if (!["--manifest", "--schema", "--previous", "--public-features"].includes(argument)) {
      console.error(`ERROR: unknown argument ${argument}`);
      process.exit(2);
    }
    const value = argv[index + 1];
    if (!value) {
      console.error(`ERROR: ${argument} requires a path`);
      process.exit(2);
    }
    index += 1;
    if (argument === "--manifest") options.manifestPath = path.resolve(value);
    if (argument === "--schema") options.schemaPath = path.resolve(value);
    if (argument === "--previous") options.previousPath = path.resolve(value);
    if (argument === "--public-features") options.publicFeaturesPath = path.resolve(value);
  }
  return options;
}

const options = parseArguments(process.argv.slice(2));
const schema = readJson(options.schemaPath, "schema");
const manifest = readJson(options.manifestPath, "manifest");
const publicFeatureSchema = readJson(publicFeatureSchemaPath, "PublicFeatureManifest schema");
const previous = options.previousPath ? readJson(options.previousPath, "previous accepted manifest") : undefined;
const publicFeatures = options.publicFeaturesPath
  ? readJson(options.publicFeaturesPath, "PublicFeatureManifest")
  : undefined;

if (options.printIdentity) {
  console.log(contentIdentity(manifest));
  process.exit(0);
}

const errors = validateManifest({ schema, publicFeatureSchema, manifest, previous, publicFeatures });
if (errors.length > 0) {
  console.error(`ADR-016 Book coverage manifest INVALID (${errors.length} error${errors.length === 1 ? "" : "s"}):`);
  for (const error of errors) console.error(`- ${error}`);
  process.exit(1);
}

const entries = Object.entries(manifest.features);
const { sections, fences } = indexIdentities(manifest);
const gated = [...fences.values()].filter((record) => record.fence.classification === "runnable-gated").length;
console.log(
  `ADR-016 Book coverage manifest OK: ${entries.length} feature entr${entries.length === 1 ? "y" : "ies"}, ` +
    `${sections.size} section${sections.size === 1 ? "" : "s"}, ${fences.size} fence${fences.size === 1 ? "" : "s"} ` +
    `(${gated} runnable-gated, ${fences.size - gated} illustrative-only), ${(manifest.flagship_fence_ids ?? []).length} flagship, ` +
    `migration ${manifest.identity_migration.kind}, content identity ${contentIdentity(manifest).slice(0, 16)}.`,
);

if (options.selfTest) {
  console.log("Forced negatives:");
  const failures = runSelfTest(schema, publicFeatureSchema, manifest);
  if (failures.length > 0) {
    console.error(`Self-test FAILED (${failures.length}):`);
    for (const failure of failures) console.error(`- ${failure}`);
    process.exit(1);
  }
  console.log("Self-test OK: every tripwire rejected its forced negative and accepted its positive control.");
}
