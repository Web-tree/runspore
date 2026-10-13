# Validation codes

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

Every problem and warning `spore validate` can report, with its help page. `spore explain <code>` prints the same page. The [validation guide](../validation.md) walks through one workflow, broken and fixed.

| Code | Severity | Title |
| --- | --- | --- |
| [`action.bad-name`](action.bad-name.md) | problem | Action name with other characters |
| [`action.bad-outcome`](action.bad-outcome.md) | problem | Not an outcome name |
| [`action.no-effect`](action.no-effect.md) | problem | Missing "effect" |
| [`action.no-kind`](action.no-kind.md) | problem | Missing "kind" |
| [`action.no-outcomes`](action.no-outcomes.md) | problem | Empty "outcomes" |
| [`action.repeated-outcome`](action.repeated-outcome.md) | problem | Outcome listed twice |
| [`action.reserved-outcome`](action.reserved-outcome.md) | problem | Outcome spore keeps for itself |
| [`action.step-field`](action.step-field.md) | problem | Step key on an action |
| [`action.unknown-effect`](action.unknown-effect.md) | problem | Unknown effect |
| [`action.unknown-function`](action.unknown-function.md) | problem | Unknown native function |
| [`action.unknown-kind`](action.unknown-kind.md) | problem | Unknown kind of action |
| [`action.unsupported-effect`](action.unsupported-effect.md) | problem | Effect this spore cannot run |
| [`command.argv-spaces`](command.argv-spaces.md) | warning | Program name with spaces |
| [`command.bad-exit-code`](command.bad-exit-code.md) | problem | Not an exit code |
| [`command.cwd-not-pointer`](command.cwd-not-pointer.md) | problem | Working folder from input not a JSON Pointer |
| [`command.exit-outcome`](command.exit-outcome.md) | problem | Exit code for an outcome the action does not list |
| [`command.no-argv`](command.no-argv.md) | problem | Missing "argv" |
| [`command.program-not-found`](command.program-not-found.md) | warning | Program not found |
| [`command.result-exit-zero`](command.result-exit-zero.md) | problem | Outcome for exit code 0 in result mode |
| [`dot.after-graph`](dot.after-graph.md) | problem | Text after the graph |
| [`dot.arrow-from-end`](dot.arrow-from-end.md) | problem | Arrow from an end |
| [`dot.attribute-typo`](dot.attribute-typo.md) | problem | Unknown attribute |
| [`dot.bad-name`](dot.bad-name.md) | problem | Workflow name with other characters |
| [`dot.bad-outcome`](dot.bad-outcome.md) | problem | Not an outcome name |
| [`dot.bad-step-name`](dot.bad-step-name.md) | problem | Step name with other characters |
| [`dot.exhausted-arrow`](dot.exhausted-arrow.md) | problem | Arrow for "exhausted" without a loop limit |
| [`dot.kind-conflict`](dot.kind-conflict.md) | problem | Two kinds for one step |
| [`dot.misplaced-attribute`](dot.misplaced-attribute.md) | problem | Attribute in the wrong place |
| [`dot.missing-arrow`](dot.missing-arrow.md) | problem | Missing arrow |
| [`dot.missing-outcome-arrow`](dot.missing-outcome-arrow.md) | problem | Missing arrow |
| [`dot.no-arrows`](dot.no-arrows.md) | problem | Signal step with no arrows |
| [`dot.no-dialect`](dot.no-dialect.md) | problem | No dialect line |
| [`dot.no-exhausted-arrow`](dot.no-exhausted-arrow.md) | problem | No arrow for "exhausted" |
| [`dot.no-name`](dot.no-name.md) | problem | No workflow name |
| [`dot.no-start`](dot.no-start.md) | problem | No first step |
| [`dot.no-steps`](dot.no-steps.md) | problem | No steps |
| [`dot.no-value`](dot.no-value.md) | problem | Attribute with no value |
| [`dot.no-way-out`](dot.no-way-out.md) | problem | No way to an end |
| [`dot.number-id`](dot.number-id.md) | problem | Unquoted number |
| [`dot.old-dialect`](dot.old-dialect.md) | problem | Old dialect |
| [`dot.repeated-arrow`](dot.repeated-arrow.md) | problem | Two arrows for one outcome |
| [`dot.repeated-attribute`](dot.repeated-attribute.md) | problem | Attribute given twice |
| [`dot.syntax`](dot.syntax.md) | problem | DOT syntax |
| [`dot.too-many-steps`](dot.too-many-steps.md) | problem | Too many steps |
| [`dot.unclosed`](dot.unclosed.md) | problem | Never closed |
| [`dot.undeclared-outcome`](dot.undeclared-outcome.md) | problem | Arrow for an outcome the action does not have |
| [`dot.unknown-attribute`](dot.unknown-attribute.md) | problem | Unknown attribute |
| [`dot.unknown-dialect`](dot.unknown-dialect.md) | problem | Unknown dialect |
| [`dot.unknown-kind`](dot.unknown-kind.md) | problem | Unknown kind |
| [`dot.unknown-start`](dot.unknown-start.md) | problem | Unknown first step |
| [`dot.unreachable`](dot.unreachable.md) | problem | Step that cannot be reached |
| [`dot.unsupported`](dot.unsupported.md) | problem | DOT that spore does not read |
| [`file.absolute`](file.absolute.md) | problem | Absolute path |
| [`file.backslash`](file.backslash.md) | problem | Backslash in a path |
| [`file.fixed-name`](file.fixed-name.md) | problem | Reference to workflow.dot or settings.json |
| [`file.missing`](file.missing.md) | problem | File not found |
| [`file.not-a-file`](file.not-a-file.md) | problem | Not a file |
| [`file.outside`](file.outside.md) | problem | File outside the folder |
| [`input.marker-no-schemas`](input.marker-no-schemas.md) | problem | Marker on a step with no schemas |
| [`input.marker-shape`](input.marker-shape.md) | problem | Marker written with something else |
| [`input.marker-too-deep`](input.marker-too-deep.md) | problem | Marker copy nested too deep |
| [`mapping.get-extra-key`](mapping.get-extra-key.md) | problem | "$get" with another key |
| [`mapping.literal-extra-key`](mapping.literal-extra-key.md) | problem | "$literal" with another key |
| [`mapping.path-not-list`](mapping.path-not-list.md) | problem | "$get" path that is not a list |
| [`mapping.path-root`](mapping.path-root.md) | problem | "$get" path with an unknown start |
| [`mapping.path-step`](mapping.path-step.md) | problem | "$get" path with a wrong step |
| [`mapping.unknown-operator`](mapping.unknown-operator.md) | problem | Unknown "$" key |
| [`node.action-field`](node.action-field.md) | problem | Action key on a step |
| [`node.bad-error-code`](node.bad-error-code.md) | problem | Not an error code |
| [`node.bad-signal`](node.bad-signal.md) | problem | Signal name with other characters |
| [`node.field-not-allowed`](node.field-not-allowed.md) | problem | Key that the step's kind does not take |
| [`node.from-dot`](node.from-dot.md) | problem | Key that comes from workflow.dot |
| [`node.limit-on-exhausted`](node.limit-on-exhausted.md) | problem | Loop limit on "exhausted" |
| [`node.limit-too-high`](node.limit-too-high.md) | problem | Loop limit too high |
| [`node.limit-unknown-outcome`](node.limit-unknown-outcome.md) | problem | Loop limit for an outcome with no arrow |
| [`node.missing-field`](node.missing-field.md) | problem | Step without what its kind needs |
| [`node.schema-reserved-outcome`](node.schema-reserved-outcome.md) | problem | Schema for an outcome with no output |
| [`node.schema-unknown-outcome`](node.schema-unknown-outcome.md) | problem | Schema for an outcome the step does not have |
| [`node.signal-schemas-differ`](node.signal-schemas-differ.md) | problem | One signal, two schemas |
| [`node.unknown-action`](node.unknown-action.md) | problem | Unknown action |
| [`schema.file-not-json`](schema.file-not-json.md) | problem | Schema file that is not JSON |
| [`schema.input-file-not-json`](schema.input-file-not-json.md) | problem | Input schema file that is not JSON |
| [`schema.input-not-object`](schema.input-not-object.md) | problem | Input schema that is not an object |
| [`schema.input-outside-subset`](schema.input-outside-subset.md) | problem | Input schema outside what spore checks |
| [`schema.outside-subset`](schema.outside-subset.md) | problem | Schema outside what spore checks |
| [`settings.empty`](settings.empty.md) | problem | Empty |
| [`settings.from-dot`](settings.from-dot.md) | problem | Key that comes from workflow.dot |
| [`settings.misplaced-action-key`](settings.misplaced-action-key.md) | problem | Key in the wrong place |
| [`settings.misplaced-key`](settings.misplaced-key.md) | problem | Key in the wrong place |
| [`settings.misplaced-step-key`](settings.misplaced-step-key.md) | problem | Key in the wrong place |
| [`settings.missing-field`](settings.missing-field.md) | problem | Missing key |
| [`settings.no-format`](settings.no-format.md) | problem | No format |
| [`settings.not-allowed`](settings.not-allowed.md) | problem | Value not allowed |
| [`settings.not-decimal`](settings.not-decimal.md) | problem | Milliseconds not written as digits |
| [`settings.not-integer`](settings.not-integer.md) | problem | Not a whole number |
| [`settings.not-json`](settings.not-json.md) | problem | Not JSON |
| [`settings.not-object`](settings.not-object.md) | problem | Settings that are not an object |
| [`settings.number-too-large`](settings.number-too-large.md) | problem | Number too large |
| [`settings.repeated-key`](settings.repeated-key.md) | problem | Key given twice |
| [`settings.step-typo`](settings.step-typo.md) | problem | Unknown step |
| [`settings.too-deep`](settings.too-deep.md) | problem | Nested too deep |
| [`settings.too-large`](settings.too-large.md) | problem | Number too large |
| [`settings.too-small`](settings.too-small.md) | problem | Number too small |
| [`settings.typo-key`](settings.typo-key.md) | problem | Unknown key |
| [`settings.unknown-key`](settings.unknown-key.md) | problem | Unknown key |
| [`settings.unknown-step`](settings.unknown-step.md) | problem | Unknown step |
| [`settings.wrong-format`](settings.wrong-format.md) | problem | Wrong format |
| [`settings.wrong-type`](settings.wrong-type.md) | problem | Wrong type |
| [`spore.broken-message`](spore.broken-message.md) | problem | spore made a broken message |
| [`spore.missed-check`](spore.missed-check.md) | problem | spore missed a check |
| [`spore.unworded-shape`](spore.unworded-shape.md) | problem | spore has no words for a shape problem |
