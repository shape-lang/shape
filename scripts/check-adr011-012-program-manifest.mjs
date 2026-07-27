#!/usr/bin/env node

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { validateJsonSchema202012 } from "./lib/adr011-012-json-schema.mjs";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDirectory, "..");
const defaultManifestPath = path.join(repositoryRoot, "docs/program/adr011-012/program-manifest.draft.json");
const defaultSchemaPath = path.join(repositoryRoot, "docs/program/adr011-012/program-manifest.schema.json");
const EXPECTED_ENTRY_COUNT = 89;
const EXPECTED_FORMULA = "93+C+D+T+L+I+P+B";
const EXPECTED_GRAPH_DIGEST = "adb65f7fb35d658097ca65b7670162a4d3ac6688b421d1473b0f8f01cb48873d";
const EXPECTED_ADRS = Array.from({ length: 6 }, (_, index) => `ADR-${String(index + 11).padStart(3, "0")}`);
const EXPECTED_RULINGS = Array.from({ length: 20 }, (_, index) => `R${index + 1}`);
const EXPECTED_TEMPLATE_VARIABLES = ["C", "D", "T", "L", "I", "P", "B"];
const ISSUE_PATTERN = /^#[1-9][0-9]*$/;
const SYMBOLIC_PATTERN = /^[A-Z][A-Z0-9]*(?:-[A-Z0-9]+)*$/;
const errors = [];

function check(condition, message) {
  if (!condition) errors.push(message);
}

function isObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function nonEmpty(value) {
  return typeof value === "string" && value.trim().length > 0;
}

function unique(values) {
  return Array.isArray(values) && new Set(values).size === values.length;
}

function sameSet(left, right) {
  return Array.isArray(left) && left.length === right.length && left.every((value) => right.includes(value));
}

function readJson(filePath, label) {
  try {
    return JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch (error) {
    console.error(`ERROR: cannot read ${label} ${filePath}: ${error.message}`);
    process.exit(1);
  }
}

function parseArguments(argv) {
  let manifestPath = defaultManifestPath;
  let schemaPath = defaultSchemaPath;
  let trackerSnapshotPath;
  let approvalViewPath;
  let selfTest = false;
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--help") {
      console.log("Usage: node scripts/check-adr011-012-program-manifest.mjs [--manifest path] [--schema path] [--tracker-snapshot path] [--approval-view path] [--self-test]");
      console.log("Tracker snapshots contain issues plus symbolic_issue_ids mappings for new-ticket IDs.");
      process.exit(0);
    }
    if (argument === "--self-test") {
      selfTest = true;
      continue;
    }
    if (!["--manifest", "--schema", "--tracker-snapshot", "--approval-view"].includes(argument)) {
      console.error(`ERROR: unknown argument ${argument}`);
      process.exit(2);
    }
    const value = argv[index + 1];
    if (!value) {
      console.error(`ERROR: ${argument} requires a path`);
      process.exit(2);
    }
    index += 1;
    if (argument === "--manifest") manifestPath = path.resolve(value);
    if (argument === "--schema") schemaPath = path.resolve(value);
    if (argument === "--tracker-snapshot") trackerSnapshotPath = path.resolve(value);
    if (argument === "--approval-view") approvalViewPath = path.resolve(value);
  }
  return { manifestPath, schemaPath, trackerSnapshotPath, approvalViewPath, selfTest };
}

function validateSchema(schema, manifest) {
  const priorErrorCount = errors.length;
  check(isObject(schema), "schema must be an object");
  check(schema?.$schema === "https://json-schema.org/draft/2020-12/schema", "schema must use JSON Schema draft 2020-12");
  check(schema?.$id === "https://shape-lang.org/schemas/adr011-012-program-manifest.v1.schema.json", "schema $id is wrong");
  check(schema?.properties?.entries?.minItems === EXPECTED_ENTRY_COUNT, "schema entry minimum is wrong");
  check(schema?.properties?.entries?.maxItems === EXPECTED_ENTRY_COUNT, "schema entry maximum is wrong");
  check(schema?.properties?.expansion_templates?.minItems === 7, "schema must require seven expansion templates");
  check(schema?.properties?.publication?.properties?.final_entry_count_formula?.const === EXPECTED_FORMULA, "schema final-count formula is wrong");
  const result = validateJsonSchema202012(schema, manifest);
  for (const error of result.errors) check(false, `JSON Schema: ${error}`);
  return result.valid && errors.length === priorErrorCount;
}

function runSchemaSelfTest(schema, manifest) {
  const mutations = [
    ["missing publication.ready_authority", (value) => delete value.publication.ready_authority],
    ["missing Book same_slice_obligations", (value) => delete value.standing_rules.public_behavior_book_gate.same_slice_obligations],
    ["illegal top-level property", (value) => { value.illegal_top_level_property = true; }],
    ["wrong manifest_version const", (value) => { value.manifest_version = 2; }],
  ];
  for (const [label, mutate] of mutations) {
    const candidate = structuredClone(manifest);
    mutate(candidate);
    check(!validateJsonSchema202012(schema, candidate).valid, `schema self-test accepted ${label}`);
  }
  const unsupportedSchema = structuredClone(schema);
  unsupportedSchema.unsupported_test_keyword = true;
  check(!validateJsonSchema202012(unsupportedSchema, manifest).valid, "schema self-test accepted an unsupported keyword");
}

function validateTopLevel(manifest) {
  check(isObject(manifest), "manifest must be an object");
  check(manifest?.$schema === "./program-manifest.schema.json", "manifest must reference the local schema");
  check(manifest?.manifest_version === 1, "manifest_version must be 1");
  check(manifest?.program_id === "adr011-012", "program_id must be adr011-012");
  check(manifest?.status === "draft-awaiting-ratification", "manifest must remain draft-awaiting-ratification");
  check(manifest?.claims?.tracker_published === false, "draft must not claim tracker publication");
  check(manifest?.claims?.implementation_started === false, "draft must not claim implementation");
  check(manifest?.claims?.evidence_complete === false, "draft must not claim complete evidence");
  check(sameSet(manifest?.authority_catalog?.adrs, EXPECTED_ADRS), "authority catalog must contain ADR-011 through ADR-016 exactly");
  check(sameSet(manifest?.authority_catalog?.rulings, EXPECTED_RULINGS), "authority catalog must contain R1 through R20 exactly");
  check(manifest?.publication?.stage === "bootstrap-and-inventory", "publication stage is wrong");
  check(manifest?.publication?.publish_now_entry_count === EXPECTED_ENTRY_COUNT, "publish-now count is wrong");
  check(manifest?.publication?.final_entry_count_formula === EXPECTED_FORMULA, "final-count formula is wrong");
  check(manifest?.publication?.template_state === "not-github-issues", "templates must not be GitHub issues");
  check(Array.isArray(manifest?.publication?.atomic_expansion_protocol) && manifest.publication.atomic_expansion_protocol.length >= 8, "atomic expansion protocol is incomplete");
}

function validateNodes(manifest) {
  const entries = Array.isArray(manifest.entries) ? manifest.entries : [];
  const external = Array.isArray(manifest.external_dependencies) ? manifest.external_dependencies : [];
  const templates = Array.isArray(manifest.expansion_templates) ? manifest.expansion_templates : [];
  check(entries.length === EXPECTED_ENTRY_COUNT, `expected ${EXPECTED_ENTRY_COUNT} entries, found ${entries.length}`);
  check(manifest?.invariants?.expected_entry_count === EXPECTED_ENTRY_COUNT, "entry-count invariant is wrong");
  check(manifest?.invariants?.final_entry_count_formula === EXPECTED_FORMULA, "count-formula invariant is wrong");
  check(external.length === 2, "external dependencies must be #22 and #58");
  check(templates.length === 7, "expected seven expansion templates");

  const nodesById = new Map();
  const entriesById = new Map();
  const adrOwners = new Map(EXPECTED_ADRS.map((id) => [id, []]));
  const rulingOwners = new Map(EXPECTED_RULINGS.map((id) => [id, []]));
  for (const node of external) {
    check(isObject(node), "each external dependency must be an object");
    check(ISSUE_PATTERN.test(node?.id ?? ""), `invalid external dependency ${node?.id}`);
    check(nonEmpty(node?.role), `${node?.id} needs an external role`);
    check(Array.isArray(node?.blocked_by) && unique(node.blocked_by), `${node?.id} external blockers must be unique`);
    check(!nodesById.has(node?.id), `duplicate node ${node?.id}`);
    nodesById.set(node?.id, node);
  }
  check(sameSet(external.map((node) => node.id), ["#22", "#58"]), "external dependency IDs differ from #22 and #58");

  entries.forEach((entry, index) => {
    const label = `entry ${index + 1}`;
    check(isObject(entry), `${label} must be an object`);
    check(entry?.ordinal === index + 1, `${label} ordinal must be ${index + 1}`);
    check(!nodesById.has(entry?.id), `duplicate node ${entry?.id}`);
    nodesById.set(entry?.id, entry);
    entriesById.set(entry?.id, entry);
    check(nonEmpty(entry?.title), `${entry?.id} needs a title`);
    check(nonEmpty(entry?.delivers), `${entry?.id} needs a deliverable`);
    check(Array.isArray(entry?.blocked_by) && unique(entry.blocked_by), `${entry?.id} blockers must be unique`);
    if (entry?.kind === "existing-issue") {
      check(ISSUE_PATTERN.test(entry?.id ?? ""), `${entry?.id} existing issue needs a numeric ID`);
      check(["amend", "preserve"].includes(entry?.disposition), `${entry?.id} existing disposition is wrong`);
    } else {
      check(entry?.kind === "new-ticket", `${entry?.id} has an invalid kind`);
      check(SYMBOLIC_PATTERN.test(entry?.id ?? ""), `${entry?.id} new ticket needs a symbolic ID`);
      check(entry?.disposition === "new", `${entry?.id} new disposition is wrong`);
    }
    const adrs = entry?.authority?.adrs;
    const rulings = entry?.authority?.rulings;
    check(Array.isArray(adrs) && adrs.length > 0 && unique(adrs), `${entry?.id} needs unique ADR owners`);
    check(Array.isArray(rulings) && rulings.length > 0 && unique(rulings), `${entry?.id} needs unique ruling owners`);
    for (const adr of adrs ?? []) {
      check(adrOwners.has(adr), `${entry?.id} references unknown ${adr}`);
      adrOwners.get(adr)?.push(entry.id);
    }
    for (const ruling of rulings ?? []) {
      check(rulingOwners.has(ruling), `${entry?.id} references unknown ${ruling}`);
      rulingOwners.get(ruling)?.push(entry.id);
    }
  });
  for (const [adr, owners] of adrOwners) check(owners.length > 0, `${adr} has no owning entry`);
  for (const [ruling, owners] of rulingOwners) check(owners.length > 0, `${ruling} has no owning entry`);
  for (const node of nodesById.values()) {
    for (const blocker of node.blocked_by ?? []) {
      check(nodesById.has(blocker), `${node.id} has unresolved blocker ${blocker}`);
      check(blocker !== node.id, `${node.id} cannot block itself`);
    }
  }

  const templateIds = new Set();
  const variables = [];
  for (const template of templates) {
    check(isObject(template), "each expansion template must be an object");
    check(SYMBOLIC_PATTERN.test(template?.id ?? ""), `invalid template ID ${template?.id}`);
    check(!templateIds.has(template?.id), `duplicate template ${template?.id}`);
    check(!nodesById.has(template?.id), `${template?.id} must not be a graph node`);
    templateIds.add(template?.id);
    variables.push(template?.count_variable);
    check(template?.state === "not-a-github-issue", `${template?.id} must remain a non-issue template`);
    check(template?.materialize_before_inventory_close === true, `${template?.id} must materialize before inventory close`);
    check(template?.child_blocked_by_inventory === true, `${template?.id} children must be blocked by their open inventory`);
    check(nonEmpty(template?.issue_pattern) && template.issue_pattern.endsWith("-*"), `${template?.id} needs an issue pattern`);
    check(nonEmpty(template?.partition_rule), `${template?.id} needs a partition rule`);
    check(nodesById.has(template?.inventory_ticket), `${template?.id} inventory ticket does not resolve`);
    check(nodesById.has(template?.capstone_ticket) || SYMBOLIC_PATTERN.test(template?.capstone_ticket ?? ""), `${template?.id} capstone is invalid`);
    check(Array.isArray(template?.current_placeholder_targets) && template.current_placeholder_targets.length > 0 && unique(template.current_placeholder_targets), `${template?.id} needs unique placeholder targets`);
    for (const target of template?.current_placeholder_targets ?? []) {
      check(nodesById.has(target), `${template?.id} placeholder target ${target} does not resolve`);
      check(nodesById.get(target)?.blocked_by?.includes(template.inventory_ticket), `${target} must currently be blocked by ${template.inventory_ticket}`);
    }
  }
  check(sameSet(variables, EXPECTED_TEMPLATE_VARIABLES), "template variables must be C,D,T,L,I,P,B exactly");
  return { entries, external, templates, nodesById, entriesById };
}

function buildGraph(nodesById) {
  const dependents = new Map();
  for (const node of nodesById.values()) {
    for (const blocker of node.blocked_by ?? []) {
      if (!dependents.has(blocker)) dependents.set(blocker, []);
      dependents.get(blocker).push(node.id);
    }
  }
  const hasDirectEdge = (blocker, blocked) => nodesById.get(blocked)?.blocked_by?.includes(blocker) ?? false;
  function hasPath(from, to) {
    const pending = [from];
    const visited = new Set();
    while (pending.length > 0) {
      const current = pending.shift();
      if (current === to) return true;
      if (visited.has(current)) continue;
      visited.add(current);
      pending.push(...(dependents.get(current) ?? []));
    }
    return false;
  }
  return { dependents, hasDirectEdge, hasPath };
}

function validateAcyclic(nodesById) {
  const state = new Map();
  function visit(id, pathToId) {
    if (state.get(id) === 2) return;
    if (state.get(id) === 1) {
      errors.push(`dependency cycle: ${[...pathToId, id].join(" -> ")}`);
      return;
    }
    state.set(id, 1);
    for (const blocker of nodesById.get(id)?.blocked_by ?? []) visit(blocker, [...pathToId, id]);
    state.set(id, 2);
  }
  for (const id of nodesById.keys()) visit(id, []);
}

function graphDigest(manifest) {
  const contract = {
    entries: manifest.entries.map(({ id, blocked_by: blockedBy }) => [id, blockedBy]),
    external: manifest.external_dependencies.map(({ id, blocked_by: blockedBy }) => [id, blockedBy]),
    templates: manifest.expansion_templates.map(({ id, issue_pattern: pattern, count_variable: count, inventory_ticket: inventory, capstone_ticket: capstone, current_placeholder_targets: targets }) => [id, pattern, count, inventory, capstone, targets]),
    book: [manifest.standing_rules.public_behavior_book_gate.bootstrap_ticket_id, manifest.standing_rules.public_behavior_book_gate.explicit_blocked_entry_ids],
  };
  return crypto.createHash("sha256").update(JSON.stringify(contract)).digest("hex");
}

function validateInvariants(manifest, state) {
  const { entries, entriesById, nodesById } = state;
  const graph = buildGraph(nodesById);
  const invariants = manifest.invariants ?? {};
  const frontier = entries.filter((entry) => entry.blocked_by.length === 0).map((entry) => entry.id);
  check(sameSet(frontier, ["AUTHORITY-BASELINE"]), `initial frontier must be AUTHORITY-BASELINE only, found ${frontier.join(", ")}`);
  check(sameSet(invariants.initial_frontier, frontier), "declared initial frontier differs from calculated frontier");
  check(sameSet(invariants.preserved_existing_issues, ["#93"]), "only #93 may be preserved");
  check(entriesById.get("#93")?.disposition === "preserve", "#93 must be preserved");
  check(invariants.program_capstone === "#23" && (graph.dependents.get("#23") ?? []).length === 0, "#23 must be the sink capstone");
  const migrationInventories = ["SEMANTIC-LEGACY-INVENTORY", "ELABORATION-LEGACY-INVENTORY", "TOOLING-EVIDENCE-INVENTORY"];
  check(sameSet(invariants.migration_inventory_tickets, migrationInventories), "migration inventory invariant is wrong");
  for (const inventory of migrationInventories) {
    check(graph.hasDirectEdge("AUTHORITY-BASELINE", inventory), `authority baseline must block ${inventory}`);
    check(graph.hasDirectEdge(inventory, "MIGRATION-GUARD"), `${inventory} must block migration guard`);
  }
  check(graph.hasDirectEdge("MIGRATION-GUARD", "#91") && graph.hasDirectEdge("MIGRATION-GUARD", "#58"), "migration guard must replace both old #90 entry gates");
  check(sameSet(entriesById.get("#90")?.blocked_by, ["MIGRATION-GUARD", "BOOK-PAIR-PROMOTE"]), "#90 close blockers are wrong");
  check(!(graph.dependents.get("#90") ?? []).some((id) => id !== "#23"), "#90 must not gate Book or semantic child work");

  const book = manifest.standing_rules?.public_behavior_book_gate;
  check(book?.enabled === true && book?.bootstrap_ticket_id === "BOOK-PAIR-PROMOTE", "Book bootstrap rule is wrong");
  const directBookDependents = entries.filter((entry) => entry.blocked_by.includes("BOOK-PAIR-PROMOTE")).map((entry) => entry.id);
  check(sameSet(directBookDependents, [...(book?.explicit_blocked_entry_ids ?? []), "#90"]), "explicit public Book blocker list differs from graph");
  check(graph.hasPath("PF-CONTRACT", "BOOK-PAIR-PROMOTE") && graph.hasPath("BOOK-CONTRACT", "BOOK-PAIR-PROMOTE"), "both manifests must precede pair promotion");
  check(graph.hasDirectEdge("PAIR-DRIFT-GUARD", "PAIR-ATTEST") && graph.hasDirectEdge("PAIR-ATTEST", "BOOK-PAIR-PROMOTE"), "attestation and promotion sequence is wrong");
  const pair = book?.pair_promotion_protocol ?? {};
  check(pair.source_manifests_carry_exact_revisions === false, "source manifests must not carry current exact revisions");
  check(pair.exact_revision_authority === "external-candidate-reports-and-attestation", "external exact-revision authority is wrong");
  check(pair.transition_identity === "signed-monotone-cas-record", "accepted-pair transition identity is wrong");
  check(pair.attestation_owns_transition_generation === false, "attestation must not own transition generation");
  check(pair.accepted_pair_pointer === "cas-to-signed-transition" && pair.accepted_pair_authority === "selected-signed-transition", "selected signed transition must be sole accepted-pair authority");
  check(pair.source_refs_are_pair_authority === false && pair.reciprocal_source_pins === false, "source refs and reciprocal pins must not be pair authority");
  check(pair.rollback === "new-higher-generation-transition-selecting-prior-attestation", "rollback must be a new monotone transition");
  const lifecycle = book?.status_lifecycle ?? {};
  check(JSON.stringify(lifecycle.ordered_states) === JSON.stringify(["planned", "experimental", "public", "deprecated", "removed"]), "public status lifecycle order is wrong");
  check(lifecycle.backward_demotion === false && lifecycle.ambiguous_status === "blocking-status-audit-gap", "status demotion or ambiguity rule is wrong");
  check(lifecycle.broken_current === "retain-status-and-owner-until-repair-or-forward-removal", "broken current features must retain status and owner");
  const migrations = book?.identity_migration ?? {};
  check(sameSet(migrations.domains, ["feature", "section", "fence"]), "identity migration must cover feature, section, and fence");
  check(migrations.total_dispositions === "unchanged-replaced-or-removed", "identity migration dispositions must be total");
  check(migrations.tombstones === "required-for-all-three-domains", "all public identity domains need tombstones");
  for (const [id, phrases] of Object.entries({
    "PF-CONTRACT": ["total feature-ID migration", "public-to-planned rejection", "no source/counterpart SHA"],
    "BOOK-CONTRACT": ["total feature/section/fence migration", "rejection of manifest-local source/counterpart revisions"],
    "GATE-CLI": ["rejects manifest-local source revisions", "external report"],
    "PAIR-PROTOCOL": ["AcceptedPairTransition", "expected-previous digest", "selected attestation", "CAS", "append-only audit"],
    "PAIR-DRIFT-GUARD": ["status demotion", "stale-writer", "rollback-audit"],
    "PAIR-ATTEST": ["no promotion generation"],
    "BOOK-PAIR-PROMOTE": ["AcceptedPairTransition CAS", "new higher-generation transition", "sole current-pair authority"],
  })) for (const phrase of phrases) check(entriesById.get(id)?.delivers.includes(phrase), `${id} must name ${phrase}`);
  check(manifest.standing_rules?.native_execution_witness?.witness_ticket_id === "NATIVE-WITNESS", "native witness ticket is wrong");

  check(!graph.hasPath("EFFECT-MIR-PROOF", "CAPABILITY-PROOF-BASE"), "effect and capability proofs must be siblings");
  check(!graph.hasPath("CAPABILITY-PROOF-BASE", "EFFECT-MIR-PROOF"), "capability and effect proofs must be siblings");
  check(graph.hasDirectEdge("EFFECT-MIR-PROOF", "#97") && graph.hasDirectEdge("CAPABILITY-PROOF-BASE", "#97"), "#97 must join both final proofs");
  check(!graph.hasDirectEdge("HOF-NATIVE-TRACER", "#97"), "HOF native work must not start-block #97");
  for (const blocker of ["#97", "NATIVE-WITNESS", "HOF-NATIVE-TRACER"]) check(graph.hasDirectEdge(blocker, "AROUND-NATIVE-CLOSE"), `${blocker} must directly block AROUND-NATIVE-CLOSE`);
  check(graph.hasDirectEdge("AROUND-NATIVE-CLOSE", "#109") && graph.hasDirectEdge("AROUND-NATIVE-CLOSE", "#23"), "native around close must block deletion and program close");
  for (const blocker of ["OUTCOME-TEARDOWN-PROOF", "EFFECT-MIR-PROOF", "CAPABILITY-PROOF-BASE"]) check(graph.hasDirectEdge(blocker, "AROUND-ASYNC"), `${blocker} must directly block AROUND-ASYNC`);

  check(invariants.artifact_codec_ticket === "#101", "#101 must own the artifact codec");
  check(graph.hasDirectEdge("#101", "VERIFIED-ARTIFACT-PERSISTENCE"), "artifact persistence must consume #101");
  check(graph.hasDirectEdge("#101", "WIRE-V3-ENVELOPE"), "wire v3 must consume #101");
  check(graph.hasDirectEdge("WIRE-V3-ENVELOPE", "JOURNAL-CORE"), "wire v3 must precede journal core");
  check(graph.hasDirectEdge("JOURNAL-CORE", "JOURNAL-HARDENING"), "journal core must precede hardening");
  check(graph.hasDirectEdge("JOURNAL-HARDENING", "#102") && !graph.hasDirectEdge("WIRE-V3-ENVELOPE", "#102"), "#102 must join at hardening without redundant wire edge");
  check(graph.hasDirectEdge("#102", "JOURNAL-OPS") && graph.hasDirectEdge("JOURNAL-HARDENING", "JOURNAL-OPS"), "journal ops blockers are wrong");
  check(graph.hasDirectEdge("#102", "DURABLE-SUPERVISOR") && graph.hasDirectEdge("JOURNAL-HARDENING", "DURABLE-SUPERVISOR"), "durable supervisor blockers are wrong");
  check(graph.hasDirectEdge("DURABLE-SUPERVISOR", "SUPERVISOR-RECOVERY-LIFECYCLE") && graph.hasDirectEdge("JOURNAL-OPS", "SUPERVISOR-RECOVERY-LIFECYCLE"), "recovery lifecycle blockers are wrong");
  check(graph.hasDirectEdge("SUPERVISOR-RECOVERY-LIFECYCLE", "JOURNAL-CAPSTONE"), "recovery lifecycle must block journal capstone");
  check(graph.hasDirectEdge("JOURNAL-CAPSTONE", "#104") && graph.hasDirectEdge("JOURNAL-CAPSTONE", "#23"), "journal capstone must block snapshot and program close");
  for (const predecessor of invariants.stabilized_runtime_predecessors ?? []) check(graph.hasPath(predecessor, "#104"), `${predecessor} must precede snapshot v8`);

  const retry = invariants.retry_identity ?? {};
  check(retry.recovery_episode_ticket === "#99" && retry.retry_commit_ticket === "#100" && retry.remote_dispatch_ticket === "#102", "retry ticket ownership is wrong");
  check(sameSet(retry.authority?.adrs, ["ADR-015"]) && sameSet(retry.authority?.rulings, ["R7", "R8", "R9"]), "retry authority is wrong");
  check(retry.same_recovery_episode === true, "retry must remain in one RecoveryEpisode");
  check(retry.fresh_attempt_id_and_next_on_retry === true, "retry must mint fresh AttemptId and Next");
  check(retry.fresh_transfer_and_admission_on_retry === true, "semantic remote retry must mint fresh transfer and admission");
  check(retry.same_transfer_only_for_same_attempt_retransmission === true, "TransferId reuse is same-attempt retransmission only");
  check(retry.prior_escrow_before_retry_commit === "settled-or-atomically-transferred", "prior escrow settlement rule is wrong");
  check(graph.hasPath("#99", "#100") && graph.hasPath("#100", "#102"), "retry sequence #99 -> #100 -> #102 is broken");
  for (const phrase of ["retry(3)=4", "absolute deadline", "fresh AttemptId", "fresh Next"]) check(entriesById.get("#100")?.delivers.includes(phrase), `#100 must name ${phrase}`);
  for (const phrase of ["append-only history", "stricter recorded absolute deadline"]) check(entriesById.get("#104")?.delivers.includes(phrase), `#104 must name ${phrase}`);

  check(graph.hasDirectEdge("INTRINSIC-INVENTORY", "#23"), "intrinsic inventory placeholder must block #23");
  check(!graph.hasPath("INTRINSIC-INVENTORY", "#110"), "intrinsic rollout must not block unrelated #110");
  check(graphDigest(manifest) === EXPECTED_GRAPH_DIGEST, `graph contract digest is ${graphDigest(manifest)}, expected ${EXPECTED_GRAPH_DIGEST}`);
  return graph;
}

function validateConditionalRemovals(manifest, graph) {
  const policies = manifest.conditional_edge_removals ?? [];
  check(policies.length === 5 && unique(policies.map((policy) => policy?.id)), "expected five unique conditional edge policies");
  for (const policy of policies) {
    check(["remove-on-publication-after-replacements", "retain-until-future-supersession"].includes(policy?.transition), `${policy?.id} transition is invalid`);
    check(Array.isArray(policy?.remove_edges) && policy.remove_edges.length > 0, `${policy?.id} needs removed edges`);
    check(Array.isArray(policy?.replacement_paths), `${policy?.id} replacement paths must be an array`);
    check(Array.isArray(policy?.retain_edges), `${policy?.id} retained edges must be an array`);
    check(Array.isArray(policy?.conditions) && policy.conditions.length > 0, `${policy?.id} needs conditions`);
    for (const edge of policy?.remove_edges ?? []) {
      const present = graph.hasDirectEdge(edge?.blocker, edge?.blocked);
      check(policy.transition === "retain-until-future-supersession" ? present : !present, `${policy?.id} removed-edge state is wrong for ${edge?.blocker} -> ${edge?.blocked}`);
    }
    for (const edge of policy?.retain_edges ?? []) check(graph.hasDirectEdge(edge?.blocker, edge?.blocked), `${policy?.id} retained edge is absent`);
    for (const replacement of policy?.replacement_paths ?? []) {
      check(Array.isArray(replacement) && replacement.length >= 2, `${policy?.id} has an invalid replacement path`);
      for (let index = 0; index + 1 < replacement.length; index += 1) check(graph.hasDirectEdge(replacement[index], replacement[index + 1]), `${policy?.id} lacks ${replacement[index]} -> ${replacement[index + 1]}`);
    }
  }
  check((manifest.ratification_forks ?? []).length >= 1, "draft needs ratification forks");
  for (const fork of manifest.ratification_forks ?? []) {
    check(fork?.status === "awaiting-ratification", `${fork?.id} must await ratification`);
    check(nonEmpty(fork?.question) && nonEmpty(fork?.recommended), `${fork?.id} needs a question and recommendation`);
  }
}

function trackerIssues(snapshot) {
  if (Array.isArray(snapshot?.issues)) return new Map(snapshot.issues.map((issue) => [issue.id, issue]));
  if (isObject(snapshot?.issues)) return new Map(Object.entries(snapshot.issues).map(([id, issue]) => [id, isObject(issue) ? { id, ...issue } : { id }]));
  errors.push("tracker snapshot issues must be an array or object");
  return new Map();
}

function compareTracker(manifest, snapshot) {
  const mappings = isObject(snapshot?.symbolic_issue_ids) ? snapshot.symbolic_issue_ids : {};
  const issues = trackerIssues(snapshot);
  const resolve = (id) => {
    if (ISSUE_PATTERN.test(id)) return id;
    const mapped = mappings[id];
    check(ISSUE_PATTERN.test(mapped ?? ""), `tracker snapshot lacks numeric mapping for ${id}`);
    return mapped;
  };
  const seen = new Set();
  for (const node of [...manifest.external_dependencies, ...manifest.entries]) {
    const issueId = resolve(node.id);
    if (!issueId) continue;
    check(!seen.has(issueId), `tracker mapping reuses ${issueId}`);
    seen.add(issueId);
    const issue = issues.get(issueId);
    check(isObject(issue), `tracker snapshot lacks ${node.id} as ${issueId}`);
    const expected = node.blocked_by.map(resolve).filter(Boolean);
    check(Array.isArray(issue?.blocked_by) && sameSet(issue.blocked_by, expected), `native blockers for ${issueId} differ from manifest ${node.id}`);
    if (Array.isArray(issue?.body_blocked_by)) check(sameSet(issue.body_blocked_by, expected), `body blockers for ${issueId} differ from native blockers`);
  }
}

function approvalSection(manifest) {
  const cells = (value) => value.replaceAll("|", "\\|").replaceAll("\n", "<br>");
  const rows = manifest.entries.map((entry) => {
    const blockers = entry.blocked_by.length ? entry.blocked_by.map((id) => `\`${id}\``).join(", ") : "—";
    return `| ${entry.ordinal} | \`${entry.id}\` | ${cells(entry.title)} | ${blockers} | ${cells(entry.delivers)} |`;
  });
  return [
    "## Exact publish-now graph",
    "",
    "| # | ID | Title | Blocked by | What it delivers |",
    "|---:|---|---|---|---|",
    ...rows,
  ].join("\n");
}

function compareApprovalView(manifest, filePath) {
  const source = fs.readFileSync(filePath, "utf8").replaceAll("\r\n", "\n");
  const start = source.indexOf("## Exact publish-now graph");
  const end = start < 0 ? -1 : source.indexOf("\n## ", start + 3);
  const actual = start < 0 ? "" : source.slice(start, end < 0 ? source.length : end).trim();
  const expected = approvalSection(manifest).trim();
  if (actual === expected) return;
  const actualLines = actual.split("\n");
  const expectedLines = expected.split("\n");
  const line = expectedLines.findIndex((value, index) => value !== actualLines[index]);
  check(false, `approval view differs at section line ${line + 1}: expected ${expectedLines[line] ?? "<end>"}, found ${actualLines[line] ?? "<end>"}`);
}

const { manifestPath, schemaPath, trackerSnapshotPath, approvalViewPath, selfTest } = parseArguments(process.argv.slice(2));
const schema = readJson(schemaPath, "schema");
const manifest = readJson(manifestPath, "manifest");
const schemaValid = validateSchema(schema, manifest);
if (schemaValid) {
  validateTopLevel(manifest);
  const state = validateNodes(manifest);
  validateAcyclic(state.nodesById);
  const graph = validateInvariants(manifest, state);
  validateConditionalRemovals(manifest, graph);
  if (trackerSnapshotPath) compareTracker(manifest, readJson(trackerSnapshotPath, "tracker snapshot"));
  if (approvalViewPath) compareApprovalView(manifest, approvalViewPath);
  if (selfTest) runSchemaSelfTest(schema, manifest);
}

if (errors.length > 0) {
  console.error(`ADR-011–016 program manifest INVALID (${errors.length} error${errors.length === 1 ? "" : "s"}):`);
  for (const error of errors) console.error(`- ${error}`);
  process.exit(1);
}

const edgeCount = [...manifest.entries, ...manifest.external_dependencies].reduce((count, node) => count + node.blocked_by.length, 0);
console.log(`ADR-011–016 program manifest OK: ${manifest.entries.length} publish-now entries, ${edgeCount} native blocker edges, ${manifest.expansion_templates.length} non-issue templates, final ${EXPECTED_FORMULA}.`);
if (selfTest) console.log("Schema negative self-test OK: 4 required mutations and 1 unsupported-keyword mutation rejected.");
