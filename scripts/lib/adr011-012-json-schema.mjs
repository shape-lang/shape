const SUPPORTED_KEYWORDS = new Set([
  "$schema",
  "$id",
  "$defs",
  "$ref",
  "title",
  "description",
  "type",
  "const",
  "enum",
  "properties",
  "propertyNames",
  "required",
  "additionalProperties",
  "minProperties",
  "maxProperties",
  "items",
  "contains",
  "minItems",
  "maxItems",
  "uniqueItems",
  "minLength",
  "pattern",
  "minimum",
  "maximum",
  "oneOf",
  "anyOf",
  "allOf",
  "not",
  "if",
  "then",
  "else",
]);

// Applicator keywords holding a single subschema, and those holding an array of
// subschemas. Both the schema audit and the instance validator walk these
// generically so adding a keyword above is a one-line change here.
const SUBSCHEMA_KEYWORDS = ["items", "contains", "propertyNames", "not", "if", "then", "else"];
const SUBSCHEMA_LIST_KEYWORDS = ["oneOf", "anyOf", "allOf"];

const JSON_TYPES = new Set([
  "array",
  "boolean",
  "integer",
  "null",
  "number",
  "object",
  "string",
]);

function isObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function pointerToken(value) {
  return value.replaceAll("~1", "/").replaceAll("~0", "~");
}

function resolveReference(root, reference) {
  if (reference === "#") return root;
  if (!reference.startsWith("#/")) return undefined;
  try {
    return decodeURIComponent(reference.slice(2))
      .split("/")
      .map(pointerToken)
      .reduce((value, token) => value?.[token], root);
  } catch {
    return undefined;
  }
}

function valuesEqual(left, right) {
  if (typeof left === "number" && typeof right === "number") return left === right;
  if (Object.is(left, right)) return true;
  if (Array.isArray(left) && Array.isArray(right)) {
    return (
      left.length === right.length &&
      left.every((value, index) => valuesEqual(value, right[index]))
    );
  }
  if (isObject(left) && isObject(right)) {
    const leftKeys = Object.keys(left).sort();
    const rightKeys = Object.keys(right).sort();
    return (
      valuesEqual(leftKeys, rightKeys) &&
      leftKeys.every((key) => valuesEqual(left[key], right[key]))
    );
  }
  return false;
}

function typeMatches(value, type) {
  if (type === "array") return Array.isArray(value);
  if (type === "integer") return typeof value === "number" && Number.isInteger(value);
  if (type === "null") return value === null;
  if (type === "number") return typeof value === "number" && Number.isFinite(value);
  if (type === "object") return isObject(value);
  return typeof value === type;
}

function auditSchema(schema, root, path, errors, activeReferences = []) {
  if (typeof schema === "boolean") return;
  if (!isObject(schema)) {
    errors.push(`${path}: schema must be an object or boolean`);
    return;
  }
  for (const keyword of Object.keys(schema)) {
    if (!SUPPORTED_KEYWORDS.has(keyword)) {
      errors.push(`${path}: unsupported schema keyword ${keyword}`);
    }
  }
  if ("$ref" in schema) {
    if (typeof schema.$ref !== "string") {
      errors.push(`${path}/$ref: must be a string`);
    } else {
      const target = resolveReference(root, schema.$ref);
      if (target === undefined) {
        errors.push(`${path}/$ref: unresolved or non-local reference ${schema.$ref}`);
      } else if (activeReferences.includes(schema.$ref)) {
        errors.push(`${path}/$ref: recursive references are unsupported`);
      } else {
        auditSchema(target, root, `${path}/$ref(${schema.$ref})`, errors, [
          ...activeReferences,
          schema.$ref,
        ]);
      }
    }
  }
  for (const keyword of ["$schema", "$id", "title", "description"]) {
    if (keyword in schema && typeof schema[keyword] !== "string") {
      errors.push(`${path}/${keyword}: must be a string`);
    }
  }
  for (const keyword of ["then", "else"]) {
    if (keyword in schema && !("if" in schema)) {
      errors.push(`${path}/${keyword}: is inert without a sibling if`);
    }
  }
  if ("type" in schema) {
    const types = Array.isArray(schema.type) ? schema.type : [schema.type];
    if (!types.length || !uniqueStrings(types) || types.some((type) => !JSON_TYPES.has(type))) {
      errors.push(`${path}/type: must contain supported unique JSON types`);
    }
  }
  if (
    "enum" in schema &&
    (!Array.isArray(schema.enum) ||
      !schema.enum.length ||
      schema.enum.some((item, index) =>
        schema.enum.slice(0, index).some((prior) => valuesEqual(item, prior)),
      ))
  ) {
    errors.push(`${path}/enum: must be a nonempty array of unique values`);
  }
  if ("required" in schema && !uniqueStrings(schema.required)) {
    errors.push(`${path}/required: must contain unique strings`);
  }
  for (const keyword of ["minItems", "maxItems", "minLength", "minProperties", "maxProperties"]) {
    if (
      keyword in schema &&
      (!Number.isInteger(schema[keyword]) || schema[keyword] < 0)
    ) {
      errors.push(`${path}/${keyword}: must be a nonnegative integer`);
    }
  }
  for (const keyword of ["minimum", "maximum"]) {
    if (keyword in schema && !Number.isFinite(schema[keyword])) {
      errors.push(`${path}/${keyword}: must be a finite number`);
    }
  }
  if ("uniqueItems" in schema && typeof schema.uniqueItems !== "boolean") {
    errors.push(`${path}/uniqueItems: must be a boolean`);
  }
  if ("pattern" in schema) {
    try {
      if (typeof schema.pattern !== "string") throw new TypeError();
      new RegExp(schema.pattern, "u");
    } catch {
      errors.push(`${path}/pattern: must be a valid regular expression`);
    }
  }
  for (const keyword of ["properties", "$defs"]) {
    if (!(keyword in schema)) continue;
    if (!isObject(schema[keyword])) {
      errors.push(`${path}/${keyword}: must be an object`);
      continue;
    }
    for (const [name, child] of Object.entries(schema[keyword])) {
      auditSchema(child, root, `${path}/${keyword}/${name}`, errors);
    }
  }
  if ("additionalProperties" in schema) {
    const additional = schema.additionalProperties;
    if (typeof additional !== "boolean" && !isObject(additional)) {
      errors.push(`${path}/additionalProperties: must be a schema or boolean`);
    } else if (isObject(additional)) {
      auditSchema(additional, root, `${path}/additionalProperties`, errors);
    }
  }
  for (const keyword of SUBSCHEMA_KEYWORDS) {
    if (keyword in schema) auditSchema(schema[keyword], root, `${path}/${keyword}`, errors);
  }
  for (const keyword of SUBSCHEMA_LIST_KEYWORDS) {
    if (!(keyword in schema)) continue;
    if (!Array.isArray(schema[keyword]) || !schema[keyword].length) {
      errors.push(`${path}/${keyword}: must be a nonempty array`);
      continue;
    }
    schema[keyword].forEach((child, index) => {
      auditSchema(child, root, `${path}/${keyword}/${index}`, errors);
    });
  }
}

function uniqueStrings(value) {
  return (
    Array.isArray(value) &&
    value.every((item) => typeof item === "string") &&
    new Set(value).size === value.length
  );
}

function validateNode(schema, value, root, instancePath, schemaPath, refStack = []) {
  if (schema === true) return [];
  if (schema === false) return [`${instancePath}: rejected by ${schemaPath}`];
  const errors = [];
  if ("$ref" in schema) {
    if (refStack.includes(schema.$ref)) {
      return [`${schemaPath}/$ref: recursive references are unsupported`];
    }
    const target = resolveReference(root, schema.$ref);
    if (target === undefined) {
      return [`${schemaPath}/$ref: unresolved reference ${schema.$ref}`];
    }
    errors.push(
      ...validateNode(
        target,
        value,
        root,
        instancePath,
        `${schemaPath}/$ref(${schema.$ref})`,
        [...refStack, schema.$ref],
      ),
    );
  }
  if ("type" in schema) {
    const types = Array.isArray(schema.type) ? schema.type : [schema.type];
    if (!types.some((type) => typeMatches(value, type))) {
      errors.push(`${instancePath}: expected type ${types.join("|")}`);
      return errors;
    }
  }
  if ("const" in schema && !valuesEqual(value, schema.const)) {
    errors.push(`${instancePath}: value differs from ${schemaPath}/const`);
  }
  if ("enum" in schema && !schema.enum.some((item) => valuesEqual(value, item))) {
    errors.push(`${instancePath}: value is not in ${schemaPath}/enum`);
  }
  if (typeof value === "string") {
    if ("minLength" in schema && [...value].length < schema.minLength) {
      errors.push(`${instancePath}: shorter than minLength ${schema.minLength}`);
    }
    if ("pattern" in schema && !new RegExp(schema.pattern, "u").test(value)) {
      errors.push(`${instancePath}: does not match pattern ${schema.pattern}`);
    }
  }
  if (typeof value === "number") {
    if ("minimum" in schema && value < schema.minimum) {
      errors.push(`${instancePath}: less than minimum ${schema.minimum}`);
    }
    if ("maximum" in schema && value > schema.maximum) {
      errors.push(`${instancePath}: greater than maximum ${schema.maximum}`);
    }
  }
  if (Array.isArray(value)) {
    if ("minItems" in schema && value.length < schema.minItems) {
      errors.push(`${instancePath}: fewer than ${schema.minItems} items`);
    }
    if ("maxItems" in schema && value.length > schema.maxItems) {
      errors.push(`${instancePath}: more than ${schema.maxItems} items`);
    }
    if (
      schema.uniqueItems &&
      value.some((item, index) =>
        value.slice(0, index).some((prior) => valuesEqual(item, prior)),
      )
    ) {
      errors.push(`${instancePath}: array items must be unique`);
    }
    if ("items" in schema) {
      value.forEach((item, index) => {
        errors.push(
          ...validateNode(
            schema.items,
            item,
            root,
            `${instancePath}/${index}`,
            `${schemaPath}/items`,
            refStack,
          ),
        );
      });
    }
    if ("contains" in schema) {
      const matched = value.some(
        (item, index) =>
          validateNode(schema.contains, item, root, `${instancePath}/${index}`, `${schemaPath}/contains`, refStack)
            .length === 0,
      );
      if (!matched) errors.push(`${instancePath}: no item matches ${schemaPath}/contains`);
    }
  }
  if (isObject(value)) {
    const propertyCount = Object.keys(value).length;
    if ("minProperties" in schema && propertyCount < schema.minProperties) {
      errors.push(`${instancePath}: fewer than ${schema.minProperties} properties`);
    }
    if ("maxProperties" in schema && propertyCount > schema.maxProperties) {
      errors.push(`${instancePath}: more than ${schema.maxProperties} properties`);
    }
    if ("propertyNames" in schema) {
      for (const name of Object.keys(value)) {
        errors.push(
          ...validateNode(
            schema.propertyNames,
            name,
            root,
            `${instancePath}/${name}`,
            `${schemaPath}/propertyNames`,
            refStack,
          ).map((error) => `${error} (property name)`),
        );
      }
    }
    for (const required of schema.required ?? []) {
      if (!Object.hasOwn(value, required)) {
        errors.push(`${instancePath}: missing required property ${required}`);
      }
    }
    for (const [name, child] of Object.entries(schema.properties ?? {})) {
      if (!Object.hasOwn(value, name)) continue;
      errors.push(
        ...validateNode(
          child,
          value[name],
          root,
          `${instancePath}/${name}`,
          `${schemaPath}/properties/${name}`,
          refStack,
        ),
      );
    }
    const known = new Set(Object.keys(schema.properties ?? {}));
    for (const name of Object.keys(value).filter((key) => !known.has(key))) {
      if (schema.additionalProperties === false) {
        errors.push(`${instancePath}/${name}: additional property is forbidden`);
      } else if (isObject(schema.additionalProperties)) {
        errors.push(
          ...validateNode(
            schema.additionalProperties,
            value[name],
            root,
            `${instancePath}/${name}`,
            `${schemaPath}/additionalProperties`,
            refStack,
          ),
        );
      }
    }
  }
  const branchPasses = (keyword, child, index) =>
    validateNode(child, value, root, instancePath, `${schemaPath}/${keyword}/${index}`, refStack).length === 0;
  if ("oneOf" in schema) {
    const matches = schema.oneOf.filter((child, index) => branchPasses("oneOf", child, index)).length;
    if (matches !== 1) {
      errors.push(`${instancePath}: oneOf matched ${matches} branches, expected 1`);
    }
  }
  if ("anyOf" in schema && !schema.anyOf.some((child, index) => branchPasses("anyOf", child, index))) {
    errors.push(`${instancePath}: anyOf matched no branch`);
  }
  if ("allOf" in schema) {
    schema.allOf.forEach((child, index) => {
      errors.push(...validateNode(child, value, root, instancePath, `${schemaPath}/allOf/${index}`, refStack));
    });
  }
  if ("not" in schema && validateNode(schema.not, value, root, instancePath, `${schemaPath}/not`, refStack).length === 0) {
    errors.push(`${instancePath}: matches ${schemaPath}/not, which must not match`);
  }
  if ("if" in schema) {
    const conditionHolds =
      validateNode(schema.if, value, root, instancePath, `${schemaPath}/if`, refStack).length === 0;
    const branch = conditionHolds ? "then" : "else";
    if (branch in schema) {
      errors.push(
        ...validateNode(schema[branch], value, root, instancePath, `${schemaPath}/${branch}`, refStack),
      );
    }
  }
  return errors;
}

export function validateJsonSchema202012(schema, value) {
  const schemaErrors = [];
  auditSchema(schema, schema, "#", schemaErrors);
  if (schemaErrors.length) return { valid: false, errors: schemaErrors };
  const errors = validateNode(schema, value, schema, "#", "#");
  return { valid: errors.length === 0, errors };
}
