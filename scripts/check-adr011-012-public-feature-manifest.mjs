#!/usr/bin/env node
//
// ADR-016 §2 / R19 — the PublicFeatureManifest contract gate (#112, PF-CONTRACT).
//
// Exit 0 = the manifest is a legal successor of the previous accepted manifest.
// Exit 1 = a contract violation. Exit 2 = the inputs are unusable.
//
// The manifest is the Shape-owned inventory of user-observable features. This
// gate owns the rules JSON Schema cannot express, all of which exist to stop one
// class of failure: making a coverage gate green by editing the denominator.
//
//   * status is DERIVED from status_basis, never asserted beside it, so a row
//     cannot claim a maturity its evidence does not support;
//   * status moves forward only, so a feature that proved hard cannot be
//     relabelled `planned` to escape its runnable-evidence obligation;
//   * every feature_id ever published stays in the inventory forever, as a live
//     row or a removed row with a tombstone, so an identity can never be reused
//     for a different meaning and a feature can never silently disappear;
//   * the manifest carries no source revision, counterpart SHA, attestation or
//     mutable verification state, so it can never become self-referential with
//     the external pair evidence that owns those facts (ADR-016 §3, §7).
//
// The last rule is enforced against the schema as well as the manifest: a future
// schema edit that DECLARES such a property fails here, before any manifest can
// carry one.
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
const defaultManifestPath = path.join(repositoryRoot, "docs/program/adr011-012/public-feature-manifest.json");
const defaultSchemaPath = path.join(repositoryRoot, "docs/program/adr011-012/public-feature-manifest.schema.json");

const EXPECTED_SCHEMA_ID = "https://shape-lang.org/schemas/public-feature-manifest.v1.schema.json";
const EXPECTED_SCHEMA_MAJOR = 1;

// ADR-016 §2. Index order is the only permitted direction of travel.
const STATUS_ORDER = ["planned", "experimental", "public", "deprecated", "removed"];

// ADR-016 §2: status is a function of the status basis. `current-executable`
// splits on declared limits, which is what separates experimental from public.
function deriveStatus(statusBasis) {
  const limits = statusBasis?.declared_limits ?? [];
  switch (statusBasis?.classification) {
    case "never-current":
      return "planned";
    case "current-executable":
      return limits.length > 0 ? "experimental" : "public";
    case "deprecation-transition":
      return "deprecated";
    case "removal-transition":
      return "removed";
    default:
      return undefined;
  }
}

// Modes that actually run code, versus modes whose observable behaviour is a
// diagnostic or a projection.
const EXECUTION_MODES = new Set([
  "vm", "jit", "cli", "provider-loopback", "provider-network", "snapshot-resume",
  "foreign-c", "foreign-python", "foreign-typescript",
]);
const OBSERVABLE_CLASSES = new Set(["negative-rejection", "structured-diagnostic", "lsp-projection"]);

// ADR-016 §3 / §7: exact revisions, counterpart hashes, attestation digests and
// mutable "last verified" state belong to the external PairCandidate,
// adapter reports and PairAttestation — never to a source manifest.
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
// identity, which ADR-016 §2 requires so the gate can compare against it.
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
    console.error(`FATAL  #112: cannot read ${label} ${filePath}: ${error.message}`);
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

// --- schema guard --------------------------------------------------------

function checkSchema(schema, push) {
  if (!isObject(schema)) return push("schema must be an object");
  if (schema.$schema !== "https://json-schema.org/draft/2020-12/schema") {
    push("schema must use JSON Schema draft 2020-12");
  }
  if (schema.$id !== EXPECTED_SCHEMA_ID) push(`schema $id must be ${EXPECTED_SCHEMA_ID}`);

  const statusEnum = schema.$defs?.feature?.properties?.status?.enum;
  if (JSON.stringify(statusEnum) !== JSON.stringify(STATUS_ORDER)) {
    push(`schema status enum must be exactly ${STATUS_ORDER.join(" -> ")}, found ${JSON.stringify(statusEnum)}`);
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

// Maps keyed by feature_id, whose keys are author-chosen identifiers rather
// than field names. A feature legitimately named `comptime.checked-shape` is not
// a `checked_at` verification field.
const IDENTIFIER_KEYED_MAPS = new Set(["features", "identity_migration/feature_ids"]);

function checkRevisionFree(manifest, push) {
  (function walk(node, pointer) {
    if (Array.isArray(node)) return node.forEach((child, index) => walk(child, `${pointer}/${index}`));
    if (isObject(node)) {
      const keysAreIdentifiers = IDENTIFIER_KEYED_MAPS.has(pointer);
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

function checkFeatures(manifest, push) {
  const features = manifest.features ?? {};
  const publicNames = new Map();

  for (const [id, feature] of Object.entries(features)) {
    const derived = deriveStatus(feature.status_basis);
    if (derived === undefined) {
      push(`${id}: status_basis.classification does not derive a status`);
    } else if (derived !== feature.status) {
      push(`${id}: status is "${feature.status}" but its evidence derives "${derived}" — status_basis (classification ${feature.status_basis?.classification}, ${feature.status_basis?.declared_limits?.length ?? 0} declared limit(s)) is the authority, not the status field`);
    }

    const priorName = publicNames.get(feature.public_name);
    if (priorName) {
      push(`${id}: public_name "${feature.public_name}" is already used by ${priorName} — two inventory rows with one public name is an ambiguous inventory (ADR-016 §2)`);
    }
    publicNames.set(feature.public_name, id);

    const modes = new Set(feature.required_modes ?? []);
    const classes = new Set(feature.required_evidence_classes ?? []);
    const dimensions = new Set(feature.required_semantic_dimensions ?? []);

    // ADR-016 §2: public and deprecated features require runnable evidence in
    // every required mode. What "runnable" means depends on the modes: a mode
    // that executes code owes a successful execution, while a compile- or
    // LSP-only feature owes the structured rejection or projection that IS its
    // observable behaviour.
    if (feature.status === "public" || feature.status === "deprecated") {
      const executes = [...modes].some((mode) => EXECUTION_MODES.has(mode));
      if (executes && !classes.has("positive-execution")) {
        push(`${id}: is ${feature.status} with executing mode(s) ${[...modes].filter((mode) => EXECUTION_MODES.has(mode)).join(", ")} but declares no positive-execution evidence`);
      }
      if (!executes && ![...classes].some((klass) => OBSERVABLE_CLASSES.has(klass))) {
        push(`${id}: is ${feature.status} with no executing mode and no observable evidence class (one of ${[...OBSERVABLE_CLASSES].join(", ")})`);
      }
    }

    if (modes.has("vm") && modes.has("jit") && !classes.has("vm-jit-parity")) {
      push(`${id}: requires both vm and jit modes but not vm-jit-parity evidence — ADR-016 §5 makes parity two real executions, so it must be a declared obligation`);
    }
    if (classes.has("native-execution") && !modes.has("jit")) {
      push(`${id}: claims native-execution evidence without a jit required mode`);
    }
    if (modes.has("snapshot-resume") && !classes.has("snapshot-resume")) {
      push(`${id}: requires snapshot-resume mode without snapshot-resume evidence`);
    }
    if (dimensions.has("script-tier") && !classes.has("ceremony-budget")) {
      push(`${id}: declares the script-tier dimension without ceremony-budget evidence — ADR-016 §10 requires a mechanically checked ceremony:none fence`);
    }

    const tombstone = feature.tombstone;
    for (const replacement of tombstone?.replacement_feature_ids ?? []) {
      if (replacement === id) push(`${id}: tombstone names itself as its replacement`);
      else if (!(replacement in features)) {
        push(`${id}: tombstone replacement ${replacement} is not in the inventory — a tombstone must name a real successor (ADR-016 §9)`);
      }
    }
  }

  // A replacement chain that loops would let a retired identity reach itself.
  for (const id of Object.keys(features)) {
    const seen = new Set([id]);
    let frontier = features[id].tombstone?.replacement_feature_ids ?? [];
    while (frontier.length > 0) {
      const next = [];
      for (const candidate of frontier) {
        if (candidate === id) {
          push(`${id}: tombstone replacement chain is cyclic`);
          frontier = [];
          break;
        }
        if (seen.has(candidate) || !(candidate in features)) continue;
        seen.add(candidate);
        next.push(...(features[candidate].tombstone?.replacement_feature_ids ?? []));
      }
      frontier = frontier.length === 0 ? [] : next;
    }
  }
}

// --- candidate-versus-previous rules ------------------------------------

function checkPair(manifest, previous, expectedMajor, push) {
  const migration = manifest.identity_migration ?? {};

  if (manifest.manifest_version !== expectedMajor) {
    push(`manifest_version must be ${expectedMajor}, found ${manifest.manifest_version}`);
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

  const candidateFeatures = manifest.features ?? {};
  const previousFeatures = previous.features ?? {};

  for (const [id, before] of Object.entries(previousFeatures)) {
    const after = candidateFeatures[id];
    if (!after) {
      push(`${id}: present in the previous accepted manifest and absent here — a published feature_id is permanent; retire it as a removed row with a tombstone instead of deleting it (ADR-016 §2)`);
      continue;
    }

    const fromIndex = STATUS_ORDER.indexOf(before.status);
    const toIndex = STATUS_ORDER.indexOf(after.status);
    if (toIndex < fromIndex) {
      push(`${id}: status moved backward ${before.status} -> ${after.status}. ADR-016 §2 is forward-only${before.status === "public" && after.status === "planned" ? "; a previously current feature cannot be relabelled planned to escape its runnable-evidence obligation" : ""}`);
    }

    // A removed row is a tombstone. Rewriting it reattaches a retired identity
    // to new meaning, which is identity reuse by another name.
    if (before.status === "removed" && canonicalJson(before) !== canonicalJson(after)) {
      push(`${id}: was removed in the previous accepted manifest and its tombstone row changed here — a retired feature_id is never reused for a different meaning (ADR-016 §2)`);
    }

    // Family is a structural classification, not prose: a live identity that
    // changes family is being repurposed rather than reworded. A feature that
    // genuinely moves family takes a new identity and leaves a tombstone.
    // `public_name` is deliberately left free — it is prose, and ADR-016 §9
    // makes the manifest diff the review surface for it.
    if (before.family !== after.family) {
      push(`${id}: family changed ${before.family} -> ${after.family} — a published feature_id is never reused for a different meaning; give the reclassified feature a new identity and leave this one as a tombstone (ADR-016 §2)`);
    }
  }

  if (migration.kind !== "schema-major") return;

  const map = migration.feature_ids ?? {};
  if (!sameSet(Object.keys(map), Object.keys(previousFeatures))) {
    const missing = Object.keys(previousFeatures).filter((id) => !(id in map));
    const extra = Object.keys(map).filter((id) => !(id in previousFeatures));
    if (missing.length > 0) {
      push(`identity_migration.feature_ids is not total: ${missing.join(", ")} ${missing.length === 1 ? "exists" : "exist"} in the previous manifest with no disposition — a schema-major change carries a complete old-to-new identity migration map (ADR-016 §2)`);
    }
    if (extra.length > 0) {
      push(`identity_migration.feature_ids names ${extra.join(", ")}, which are not in the previous manifest`);
    }
  }

  for (const [id, disposition] of Object.entries(map)) {
    const after = candidateFeatures[id];
    if (!after) continue;
    const newIds = disposition.new_ids ?? [];
    if (disposition.action === "unchanged") {
      if (!sameSet(newIds, [id])) push(`${id}: an unchanged identity must map to itself, found ${newIds.join(", ") || "nothing"}`);
    } else if (disposition.action === "replaced") {
      if (after.status !== "removed") {
        push(`${id}: replaced by ${newIds.join(", ")} but its row is still ${after.status} — the old identifier becomes a tombstone (ADR-016 §2)`);
      }
      if (!sameSet(after.tombstone?.replacement_feature_ids ?? [], newIds)) {
        push(`${id}: tombstone replacements ${(after.tombstone?.replacement_feature_ids ?? []).join(", ") || "none"} differ from the migration map's ${newIds.join(", ")}`);
      }
      for (const newId of newIds) {
        if (newId in previousFeatures) {
          push(`${id}: replacement ${newId} already existed in the previous manifest — a replacement is a new identity, not a reused one`);
        } else if (!(newId in candidateFeatures)) {
          push(`${id}: replacement ${newId} is not in this manifest`);
        }
      }
    } else if (disposition.action === "removed") {
      if (after.status !== "removed") push(`${id}: migrated as removed but its row is ${after.status}`);
      if ((after.tombstone?.replacement_feature_ids ?? []).length > 0) {
        push(`${id}: migrated as removed but its tombstone names a replacement — use action "replaced" instead`);
      }
    }
  }
}

// --- engine --------------------------------------------------------------

export function validateManifest({ schema, manifest, previous, expectedMajor = EXPECTED_SCHEMA_MAJOR }) {
  const errors = [];
  const push = (message) => errors.push(message);

  checkSchema(schema, push);
  const schemaResult = validateJsonSchema202012(schema, manifest);
  for (const error of schemaResult.errors) push(`JSON Schema: ${error}`);
  if (previous) {
    for (const error of validateJsonSchema202012(schema, previous).errors) {
      push(`JSON Schema (previous accepted manifest): ${error}`);
    }
  }
  if (errors.length > 0) return errors;

  if (manifest.$schema !== "./public-feature-manifest.schema.json") {
    push("manifest must reference the local schema as ./public-feature-manifest.schema.json");
  }
  checkRevisionFree(manifest, push);
  checkFeatures(manifest, push);
  checkPair(manifest, previous, expectedMajor, push);
  return errors;
}

// --- forced negatives ----------------------------------------------------

// Each tripwire mutates a legal input and asserts rejection. `expect` names the
// substring the failure must mention, so a rejection for an unrelated reason
// does not count as the tripwire firing.
// A synthetic row lets the identity and status tripwires run against a fixed
// shape rather than whatever the real inventory happens to contain today.
function fixtureRow(overrides) {
  return {
    public_name: "Self-test fixture",
    family: "self-test",
    status: "public",
    status_basis: {
      classification: "current-executable",
      evidence: [{ kind: "adr", reference: "ADR-016" }],
    },
    authority: [{ kind: "adr", reference: "ADR-016" }],
    owner: { repository: "shape-lang/shape", component: "self-test fixture" },
    surfaces: ["language-syntax"],
    targets: ["expression"],
    required_modes: ["compile", "vm"],
    required_evidence_classes: ["positive-execution"],
    required_semantic_dimensions: ["user-model"],
    distributed_semantics_required: false,
    ...overrides,
  };
}

function tripwires(schema, manifest) {
  const liveId = Object.keys(manifest.features).find((id) => manifest.features[id].status !== "removed");

  const withPublicRow = () => {
    const base = structuredClone(manifest);
    base.features["selftest.current-surface"] = fixtureRow({ public_name: "Current surface (self-test fixture)" });
    return base;
  };

  const withRemovedRow = () => {
    const base = structuredClone(manifest);
    base.features["legacy.retired-surface"] = {
      public_name: "Retired surface (self-test fixture)",
      family: "self-test",
      status: "removed",
      status_basis: {
        classification: "removal-transition",
        evidence: [{ kind: "adr", reference: "ADR-016" }],
      },
      authority: [{ kind: "adr", reference: "ADR-016" }],
      owner: { repository: "shape-lang/shape", component: "self-test fixture" },
      surfaces: ["language-syntax"],
      targets: ["expression"],
      required_modes: ["compile"],
      required_evidence_classes: ["negative-rejection"],
      required_semantic_dimensions: ["user-model", "migration"],
      distributed_semantics_required: false,
      tombstone: {
        removed_in: "v0.4.0",
        reason: "self-test fixture",
        authority: { kind: "adr", reference: "ADR-016" },
      },
    };
    return base;
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

  return [
    {
      id: "T1 public -> planned",
      previous: withPublicRow(),
      candidate: successorOf(withPublicRow(), (value) => {
        value.features["selftest.current-surface"].status = "planned";
        value.features["selftest.current-surface"].status_basis.classification = "never-current";
        value.features["selftest.current-surface"].required_modes = [];
        value.features["selftest.current-surface"].required_evidence_classes = [];
      }),
      expect: "status moved backward public -> planned",
    },
    {
      id: "T2a reused feature ID — retired identity reattached",
      previous: withRemovedRow(),
      candidate: successorOf(withRemovedRow(), (value) => {
        value.features["legacy.retired-surface"].public_name = "A completely different feature";
      }),
      expect: "never reused for a different meaning",
    },
    {
      id: "T2b reused feature ID — live identity repurposed",
      previous: withPublicRow(),
      candidate: successorOf(withPublicRow(), (value) => {
        value.features["selftest.current-surface"].family = "something-else";
        value.features["selftest.current-surface"].public_name = "A completely different feature";
      }),
      expect: "family changed self-test -> something-else",
    },
    {
      id: "T3a manifest carrying a source SHA",
      previous: undefined,
      candidate: structuredClone({ ...manifest, source_sha: "0".repeat(40) }),
      expect: "additional property is forbidden",
    },
    {
      id: "T3b digest smuggled into a text field",
      previous: undefined,
      candidate: (() => {
        const value = structuredClone(manifest);
        value.features[liveId].status_basis.evidence[0].reference = `source@${"a1b2c3d4".repeat(8)}`;
        return value;
      })(),
      expect: "bare 64-character digest",
    },
    {
      id: "T3c schema declaring an attestation field",
      schema: (() => {
        const value = structuredClone(schema);
        value.$defs.feature.properties.attestation_digest = { type: "string" };
        return value;
      })(),
      previous: undefined,
      candidate: manifest,
      expect: "schema declares forbidden property attestation_digest",
    },
    // T4: the load-bearing row's status is a function of its evidence. Proven by
    // substitution: leave status_basis alone and every other status is rejected,
    // so the declared status is the only one its evidence admits.
    ...STATUS_ORDER.filter((status) => status !== manifest.features[liveId].status).map((status) => ({
      id: `T4 ${liveId} status substituted with "${status}"`,
      previous: undefined,
      candidate: (() => {
        const value = structuredClone(manifest);
        value.features[liveId].status = status;
        return value;
      })(),
      expect: liveId,
    })),
    {
      id: "T5a schema-major migration map is not total",
      previous: withRemovedRow(),
      candidate: (() => {
        const previous = withRemovedRow();
        const value = structuredClone(previous);
        value.manifest_version = previous.manifest_version + 1;
        value.identity_migration = {
          kind: "schema-major",
          previous_manifest: { schema_major: previous.manifest_version, sha256: contentIdentity(previous) },
          feature_ids: {
            [liveId]: { action: "unchanged", new_ids: [liveId], authority: { kind: "adr", reference: "ADR-016" } },
          },
        };
        return value;
      })(),
      expectedMajor: manifest.manifest_version + 1,
      expect: "is not total",
    },
    {
      id: "T5b feature dropped instead of tombstoned",
      previous: withRemovedRow(),
      candidate: successorOf(withRemovedRow(), (value) => {
        delete value.features["legacy.retired-surface"];
      }),
      expect: "a published feature_id is permanent",
    },
  ];
}

function runSelfTest(schema, manifest) {
  const failures = [];
  for (const tripwire of tripwires(schema, manifest)) {
    const activeSchema = tripwire.schema ?? schema;
    const expectedMajor = tripwire.expectedMajor ?? EXPECTED_SCHEMA_MAJOR;

    // Positive control: the same pair without the mutation must be accepted, so
    // a gate that rejects everything cannot pass its own tripwires.
    const controlErrors = validateManifest({
      schema,
      manifest: tripwire.previous
        ? (() => {
            const value = structuredClone(tripwire.previous);
            value.identity_migration = {
              kind: "revision",
              previous_manifest: { schema_major: value.manifest_version, sha256: contentIdentity(tripwire.previous) },
            };
            return value;
          })()
        : manifest,
      previous: tripwire.previous,
      expectedMajor: EXPECTED_SCHEMA_MAJOR,
    });
    if (controlErrors.length > 0) {
      failures.push(`${tripwire.id}: positive control was rejected — ${controlErrors[0]}`);
    }

    const errors = validateManifest({
      schema: activeSchema,
      manifest: tripwire.candidate,
      previous: tripwire.previous,
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
  const options = { manifestPath: defaultManifestPath, schemaPath: defaultSchemaPath, selfTest: false, printIdentity: false };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--help") {
      console.log("Usage: node scripts/check-adr011-012-public-feature-manifest.mjs [--manifest path] [--schema path] [--previous path] [--self-test] [--print-identity]");
      console.log("");
      console.log("  --previous        the previous ACCEPTED manifest to validate against. Omit only");
      console.log("                    while identity_migration.kind is \"initial\".");
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
    if (!["--manifest", "--schema", "--previous"].includes(argument)) {
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
  }
  return options;
}

const options = parseArguments(process.argv.slice(2));
const schema = readJson(options.schemaPath, "schema");
const manifest = readJson(options.manifestPath, "manifest");
const previous = options.previousPath ? readJson(options.previousPath, "previous accepted manifest") : undefined;

if (options.printIdentity) {
  console.log(contentIdentity(manifest));
  process.exit(0);
}

const errors = validateManifest({ schema, manifest, previous });
if (errors.length > 0) {
  console.error(`ADR-016 public feature manifest INVALID (${errors.length} error${errors.length === 1 ? "" : "s"}):`);
  for (const error of errors) console.error(`- ${error}`);
  process.exit(1);
}

const rows = Object.values(manifest.features);
const byStatus = STATUS_ORDER.map((status) => `${rows.filter((row) => row.status === status).length} ${status}`).join(", ");
console.log(`ADR-016 public feature manifest OK: ${rows.length} feature row${rows.length === 1 ? "" : "s"} (${byStatus}), migration ${manifest.identity_migration.kind}, content identity ${contentIdentity(manifest).slice(0, 16)}.`);

if (options.selfTest) {
  console.log("Forced negatives:");
  const failures = runSelfTest(schema, manifest);
  if (failures.length > 0) {
    console.error(`Self-test FAILED (${failures.length}):`);
    for (const failure of failures) console.error(`- ${failure}`);
    process.exit(1);
  }
  console.log("Self-test OK: every tripwire rejected its forced negative and accepted its positive control.");
}
