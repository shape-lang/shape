const SUPPORTED_KEYWORDS = new Set([
  "$schema",
  "$id",
  "$defs",
  "$ref",
  "title",
  "type",
  "const",
  "enum",
  "properties",
  "required",
  "additionalProperties",
  "items",
  "minItems",
  "maxItems",
  "uniqueItems",
  "minLength",
  "pattern",
  "minimum",
  "maximum",
  "oneOf",
]);

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
  for (const keyword of ["$schema", "$id", "title"]) {
    if (keyword in schema && typeof schema[keyword] !== "string") {
      errors.push(`${path}/${keyword}: must be a string`);
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
  for (const keyword of ["minItems", "maxItems", "minLength"]) {
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
  if ("items" in schema) {
    auditSchema(schema.items, root, `${path}/items`, errors);
  }
  if ("oneOf" in schema) {
    if (!Array.isArray(schema.oneOf) || !schema.oneOf.length) {
      errors.push(`${path}/oneOf: must be a nonempty array`);
    } else {
      schema.oneOf.forEach((child, index) => {
        auditSchema(child, root, `${path}/oneOf/${index}`, errors);
      });
    }
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
  }
  if (isObject(value)) {
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
  if ("oneOf" in schema) {
    const matches = schema.oneOf.filter(
      (child, index) =>
        validateNode(
          child,
          value,
          root,
          instancePath,
          `${schemaPath}/oneOf/${index}`,
          refStack,
        ).length === 0,
    ).length;
    if (matches !== 1) {
      errors.push(`${instancePath}: oneOf matched ${matches} branches, expected 1`);
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
