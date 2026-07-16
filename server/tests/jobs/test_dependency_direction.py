from __future__ import annotations

import ast
from pathlib import Path
import unittest


JOBS_ROOT = Path(__file__).resolve().parents[2] / "src" / "yap_server" / "jobs"
CONCRETE_POOL_MODULE = "yap_server.pools.batch_asr"


class JobDependencyDirectionTests(unittest.TestCase):
    def test_job_domain_owners_depend_on_the_batch_contract_not_its_implementation(
        self,
    ) -> None:
        violations: list[str] = []
        domain_paths = sorted(
            path for path in JOBS_ROOT.rglob("*.py") if path.name != "runtime.py"
        )
        for path in domain_paths:
            name = path.relative_to(JOBS_ROOT).as_posix()
            tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
            for node in ast.walk(tree):
                module = node.module if isinstance(node, ast.ImportFrom) else None
                if module == CONCRETE_POOL_MODULE:
                    violations.append(f"{name}:{node.lineno}")
                if isinstance(node, ast.Import):
                    violations.extend(
                        f"{name}:{node.lineno}"
                        for alias in node.names
                        if alias.name == CONCRETE_POOL_MODULE
                    )

        self.assertEqual(
            violations,
            [],
            "job-domain owners imported the concrete batch pool facade",
        )


if __name__ == "__main__":
    unittest.main()
