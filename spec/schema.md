# Output schemas 0.2

A workflow may declare an **output schema** for each way a step can end: the shape
the step's data must have when it ends with that outcome. A node carries them in
`outputSchemas`, an object from outcome to schema ([kernel.md](kernel.md) §2, W13).
The host checks every result against the schema of its outcome the moment the result
arrives ([host.md](host.md) §2, §3). An outcome with no schema, and a node with none,
is not checked.

The schema language is a subset of JSON Schema 2020-12. The crate
`runspore-schema` implements it; the build and the host share it.

## 1. The subset

A schema is a JSON object using only these keywords, with their JSON Schema 2020-12
meaning:

| Keyword | Rule |
| --- | --- |
| `type` | One of `object`, `array`, `string`, `integer`, `boolean`, `null`. A single string, not a list: a union is written with `anyOf`. `number` is refused: data holds integers only ([kernel.md](kernel.md) §1). |
| `properties`, `required` | Only beside `"type": "object"`. `properties` maps a name to a schema. `required` lists names, each once, each a key of `properties`. |
| `additionalProperties` | Only `false`, only beside `"type": "object"`. Absent, a value may carry keys `properties` does not name. |
| `items` | Only beside `"type": "array"`: one schema, which every item must fit. |
| `enum`, `const` | Strings, integers, booleans or `null`. An `enum` list is non-empty and holds no value twice. |
| `anyOf` | A non-empty list of schemas. No keyword sits beside it except `description` and `title`. |
| `minLength`, `maxLength` | Only beside `"type": "string"`. Integers from 0 to 65536. Lengths count Unicode code points. |
| `minItems`, `maxItems` | Only beside `"type": "array"`. Integers from 0 to 65536. |
| `minimum`, `maximum` | Only beside `"type": "integer"`. Integers within ±(2^53 − 1). |
| `description`, `title` | Strings. Not checked against data. |

A lower bound above its upper bound (`minLength` over `maxLength`, `minItems` over
`maxItems`, `minimum` over `maximum`) admits no value and is refused.

Any other keyword is refused, among them `pattern`, `format`, `oneOf`, `allOf`, `not`,
`if`, `then`, `else`, `$schema` and `$id`. `$defs` and `$ref` exist only in the
authoring form (§2): a package holds none.

### Depth

A schema is part of the package, so the package's depth limit (32 levels of objects
and arrays, [kernel.md](kernel.md) §1) counts from the package root, not from the
schema. The root object of a schema in `outputSchemas` is the fifth level (the
package, `nodes`, the node, `outputSchemas`, the schema), which leaves the schema 28
levels. Each nested property uses two (`properties`, then the property's schema); a
`required`, `enum` or `anyOf` list uses one. A schema that does not fit is refused.

### Size

A schema must admit a value that fits through the 64 KiB result limit
([host.md](host.md) §3). The smallest serialized size of any value a schema admits is
computed as follows, and a schema whose smallest size exceeds 65536 bytes is refused:

| Schema | Smallest size |
| --- | --- |
| `anyOf` | The smallest of its branches |
| `const` | The size of its value |
| `enum` | The size of its shortest value |
| `type: "null"`, `type: "boolean"` | 4 |
| `type: "integer"` | 1 when the bounds admit 0; else the length of the bound nearer 0 |
| `type: "string"` | 2 + `minLength` |
| `type: "array"` | 2 when `minItems` is 0 or absent; else 2 + `minItems` × (smallest item + 1) − 1, an item without `items` counting 1 |
| `type: "object"` | 2 + the sum over `required` names of (the name's escaped length + 3 + its property's smallest size), + one comma between each two |
| no `type` | 1 |

The bound is a lower limit only: a schema that passes may still be filled with data
too large to report, which the result limit catches.

## 2. The authoring form

In a workflow folder ([dot.md](dot.md) §4) a schema may also use:

- `$defs` at the schema's root: an object of named schemas.
- `$ref` with the value `#/$defs/<name>`, where `<name>` is a key of `$defs`. A
  `$ref` stands alone, except for `description` and `title`, which replace the
  definition's own.

The build replaces every `$ref` with a copy of the definition it names and removes
`$defs`, so the package, and with it the run's identity, holds the full schema. A
reference that leads back into a definition being expanded is refused: a schema may
not recurse. A reference to anything but `#/$defs/<name>`, such as another file or a
URL, is refused.

## 3. Checking data

Data fits a schema when it satisfies every keyword, as in JSON Schema 2020-12:
`type` names the data's type (an integer is `integer`, never `number`), `anyOf` needs
one branch to fit, a `required` name must be a key, and with
`"additionalProperties": false` every key must be named in `properties`.

A check collects every place where the data does not fit, up to 16, rather than
stopping at the first. Each is a **data error**: a `path` and a `message`.

- `path` is a JSON Pointer (RFC 6901) into the data, such as `/requests/2/file`, with
  `""` for the whole value. `~` is written `~0` and `/` is written `~1`, so no key is
  ambiguous. A missing required property is reported at the property's own path.
- `message` says what was expected and what arrived, for example
  `expected type "string", got integer (42)`, `required, but missing`, or
  `expected at least 1 items, got 0`. A string is shown up to 40 characters; an
  object or array by its type alone.

Data that does not fit is refused with the code `output.invalid` and the details
`{"outcome": "<outcome>", "errors": [{"path": "…", "message": "…"}, …]}`. Printed for
people, the errors come one per line, path first, with `(root)` standing for `""`:

```text
/requests/1/file: expected type "string", got integer (42)
(root): expected type "object", got null
```

Missing data (a signal without data, a result without output) is `null`.

## 4. Errors

| Code | Where | Cause |
| --- | --- | --- |
| `workflow.invalid-schema` | `spore build`, `validate`, `start`; the host's `validate_workflow` | A schema outside the subset, too deep, too large (§1), or with a bad reference (§2). The error names the node, the outcome and the JSON Pointer of the offending keyword within the schema |
| `output.invalid` | A step's result, a signal, a resolution | Data that does not fit its outcome's schema (§3) |
