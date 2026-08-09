#!/usr/bin/env python3
"""Direct regression coverage for validate-panic-policy.py."""

from __future__ import annotations

import importlib.util
import pathlib
import sys
import unittest


SCRIPT = pathlib.Path(__file__).with_name("validate-panic-policy.py")
SPEC = importlib.util.spec_from_file_location("validate_panic_policy", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"cannot load {SCRIPT}")
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class PanicPolicyRegressionTests(unittest.TestCase):
    def test_added_production_panic_is_reported_with_new_line_number(self) -> None:
        diff = """\
diff --git a/crates/demo/src/lib.rs b/crates/demo/src/lib.rs
@@ -4,0 +5,2 @@
+    let value = input.unwrap()
+    // expect(\"not a panic path\")
"""

        findings = MODULE.findings_from_diff(diff)

        self.assertEqual(len(findings), 1)
        self.assertEqual(findings[0].path, "crates/demo/src/lib.rs")
        self.assertEqual(findings[0].line, 5)
        self.assertEqual(findings[0].kind, "unwrap call")

    def test_removed_and_non_production_panics_are_ignored(self) -> None:
        diff = """\
diff --git a/crates/demo/tests/policy.rs b/crates/demo/tests/policy.rs
@@ -1,2 +1,2 @@
-    panic!(\"old path\")
+    assert!(true)
diff --git a/examples/demo/src/main.rs b/examples/demo/src/main.rs
@@ -1,0 +2 @@
+    unreachable!()
"""

        self.assertEqual(MODULE.findings_from_diff(diff), [])


if __name__ == "__main__":
    unittest.main()
