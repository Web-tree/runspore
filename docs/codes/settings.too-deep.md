# `settings.too-deep`: Nested too deep

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate settings.too-deep` reports:

```text
── Nested too deep ────────────────── settings.too-deep/settings.json:42  settings.too-deep
"nodes" → "build" → "input" has values nested deeper than the 32 levels a package may hold.

 42 │                                                                 "a": { "a": "x" }
    │                                                                      ^^^^^^^^^^^^

To fix: flatten it.
```

The fix changes:

```text
settings.too-deep/settings.json:
 11 │-    "build": {
 12 │-      "action": "build",
 13 │-      "input": {
 14 │-        "a": {
 15 │-          "a": {
 16 │-            "a": {
 17 │-              "a": {
 18 │-                "a": {
 19 │-                  "a": {
 20 │-                    "a": {
 21 │-                      "a": {
 22 │-                        "a": {
 23 │-                          "a": {
 24 │-                            "a": {
 25 │-                              "a": {
 26 │-                                "a": {
 27 │-                                  "a": {
 28 │-                                    "a": {
 29 │-                                      "a": {
 30 │-                                        "a": {
 31 │-                                          "a": {
 32 │-                                            "a": {
 33 │-                                              "a": {
 34 │-                                                "a": {
 35 │-                                                  "a": {
 36 │-                                                    "a": {
 37 │-                                                      "a": {
 38 │-                                                        "a": {
 39 │-                                                          "a": {
 40 │-                                                            "a": {
 41 │-                                                              "a": {
 42 │-                                                                "a": { "a": "x" }
 43 │-                                                              }
 44 │-                                                            }
 45 │-                                                          }
 46 │-                                                        }
 47 │-                                                      }
 48 │-                                                    }
 49 │-                                                  }
 50 │-                                                }
 51 │-                                              }
 52 │-                                            }
 53 │-                                          }
 54 │-                                        }
 55 │-                                      }
 56 │-                                    }
 57 │-                                  }
 58 │-                                }
 59 │-                              }
 60 │-                            }
 61 │-                          }
 62 │-                        }
 63 │-                      }
 64 │-                    }
 65 │-                  }
 66 │-                }
 67 │-              }
 68 │-            }
 69 │-          }
 70 │-        }
 71 │-      }
 72 │-    }
 11 │+    "build": { "action": "build", "input": { "a": "x" } }
```

The example folder is [`catalogue/examples/settings.too-deep`](../../crates/runspore-build/catalogue/examples/settings.too-deep).
