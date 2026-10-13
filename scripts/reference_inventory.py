#!/usr/bin/env -S uv run
"""Audit every executable reference case and its assertion sites against Rust tests."""

import argparse
import hashlib
import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parents[1]
LEDGER = ROOT / "tests/fixtures/reference-cases.json"
ADAPTER = "src/tests/adapters.rs::"
WORKFLOW = "src/tests/workflows.rs::"
PURE = "src/tests/unit/pure.rs::"
GOLDEN = "src/tests/unit/reference.rs::reference_goldens"

# These cases exercise boundaries, or document decisions without calling a pure function.
BOUNDARIES = {
    "TestDefaultConfig": [
        ADAPTER + "github_lookup_fallback_errors_creation_and_status_diagnostics",
        PURE + "cli_preserves_go_boundaries_and_boolean_values",
    ],
    "TestValidateRemoteRefRejectsDetachedHeadPlaceholder": [
        ADAPTER + "git_ref_validation_rewrite_failures_and_missing_local_branch"
    ],
    "TestDraftPRCreationLogic": [ADAPTER + "github_owner_query_and_all_draft_creation_cases"],
    "TestDraftFlagPrecedence": [ADAPTER + "github_owner_query_and_all_draft_creation_cases"],
    "TestPRNumberExtraction": [
        ADAPTER + "github_lookup_fallback_errors_creation_and_status_diagnostics"
    ],
    "TestGithubRepoOwner": [ADAPTER + "github_owner_query_and_all_draft_creation_cases"],
    "TestCommitNewlyCreatedFlag": [
        ADAPTER + "github_lookup_fallback_errors_creation_and_status_diagnostics"
    ],
    "TestDraftStatusPreservation": [
        WORKFLOW + "existing_drafts_and_manually_ready_prs_preserve_their_state",
        WORKFLOW + "first_repeat_and_unchanged_submission_preserve_bases_bodies_and_draft_choice",
    ],
    "TestDraftPRCreationVsUpdateBehavior": [
        WORKFLOW + "first_repeat_and_unchanged_submission_preserve_bases_bodies_and_draft_choice"
    ],
    "TestOutputFieldsAndReplacementsInSync": [
        PURE + "output_presets_custom_templates_and_validation"
    ],
    "TestLoadPRTemplate": [ADAPTER + "templates_precedence_and_submission_cache_boundary"],
    "TestGetPRTemplateCaching": [
        WORKFLOW + "template_is_loaded_once_per_submission_even_when_file_changes"
    ],
    "TestPushRemoteRef": [
        WORKFLOW + "failures_preserve_completed_pushes_and_report_exact_postcondition",
        ADAPTER + "git_ref_validation_rewrite_failures_and_missing_local_branch",
    ],
    "TestConfirm": [
        WORKFLOW + "native_stack_configuration_authorises_linking_and_rebuild_remains_explicit",
        "src/tests/progress.rs::real_terminal_and_redirected_output_preserve_prompts_and_results",
    ],
}


# The application now delegates configuration to Git rather than standalone files.
SUPERSEDED_CONFIGURATION = {
    "TestReadConfigFile_JSONAndYAMLParseIdentically",
    "TestReadConfigFile_JSONPreferredOverYAML",
    "TestReadConfigFile_MalformedTreatedAsAbsent",
    "TestFindNearestProjectConfigDir_NearestWins",
    "TestFindNearestProjectConfigDir_StopsAtRepoRoot_SubmoduleBoundary",
    "TestLoadFileConfig_Precedence_GlobalProjectLocal",
    "TestLoadFileConfig_NoConfigIsEmpty",
    "TestLoadFileConfig_OutputPrecedence_GlobalProjectLocal",
    "TestLoadFileConfig_OutputNearestProjectWins",
}


def inventory(source: pathlib.Path, results: pathlib.Path) -> dict:
    """Associate executed subcases, source assertion sites and recorded production calls."""
    events = [json.loads(line) for line in results.read_text().splitlines()]
    cases = sorted(
        {event["Test"] for event in events if event.get("Action") == "pass" and "Test" in event}
    )
    leaves = [case for case in cases if not any(other.startswith(case + "/") for other in cases)]
    fixtures = [
        json.loads(line)
        for line in (ROOT / "tests/fixtures/reference.jsonl").read_text().splitlines()
    ]
    functions = []
    for path in sorted(source.glob("*_test.go")):
        text = path.read_text()
        declarations = list(re.finditer(r"^func (Test\w+)\(t \*testing.T\)", text, re.M))
        for index, declaration in enumerate(declarations):
            name = declaration[1]
            end = declarations[index + 1].start() if index + 1 < len(declarations) else len(text)
            body = text[declaration.start() : end]
            first_line = text[: declaration.start()].count("\n") + 1
            assertions = [
                {"line": first_line + offset, "statement": line.strip()}
                for offset, line in enumerate(body.splitlines())
                if re.search(r"\bt\.(?:Errorf?|Fatalf?|FailNow|Fail)\(", line)
            ]
            mapped = []
            for case in [case for case in leaves if case.split("/")[0] == name]:
                golden_lines = [
                    index + 1 for index, fixture in enumerate(fixtures) if fixture["case"] == case
                ]
                tests = ([GOLDEN] if golden_lines else []) + BOUNDARIES.get(name, [])
                superseded = name in SUPERSEDED_CONFIGURATION
                if superseded:
                    tests.extend(
                        [
                            ADAPTER + "git_configuration_scopes_includes_and_empty_overrides",
                            ADAPTER + "standalone_configuration_files_are_not_application_settings",
                        ]
                    )
                if not tests:
                    raise AssertionError(f"Unmapped reference case: {case}")
                entry = {"case": case, "golden_lines": golden_lines, "rust_tests": tests}
                if superseded:
                    entry["superseded_contract"] = (
                        "Standalone configuration files were replaced by native Git configuration; "
                        "the mapped tests verify the replacement, not the original file semantics."
                    )
                mapped.append(entry)
            if not mapped:
                raise AssertionError(f"Reference function was not executed: {name}")
            functions.append(
                {
                    "suite": path.name,
                    "source_sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                    "function": name,
                    "line": first_line,
                    "assertion_sites": assertions,
                    "cases": mapped,
                }
            )
    if len(functions) != 74 or sum(len(item["cases"]) for item in functions) != len(leaves):
        raise AssertionError("Reference inventory changed; review the compatibility contract")
    return {
        "revision": "132dc66ea004d03606585c9bdfe95289bd0b9dcb",
        "function_count": len(functions),
        "case_count": len(leaves),
        "golden_call_count": len(fixtures),
        "superseded_configuration_functions": sorted(SUPERSEDED_CONFIGURATION),
        "functions": functions,
    }


def verify(document: dict) -> None:
    """Check that every mapped Rust test or executable scenario remains present."""
    for function in document["functions"]:
        for case in function["cases"]:
            for target in case["rust_tests"]:
                filename, symbol = target.split("::", 1)
                source = (ROOT / filename).read_text()
                expected = f'"{symbol}":' if filename.endswith(".py") else f"fn {symbol}("
                if expected not in source:
                    raise AssertionError(f"Missing {target} for {case['case']}")
    print(
        f"Mapped {document['function_count']} reference functions, {document['case_count']} leaf cases and {document['golden_call_count']} captured production calls; {len(SUPERSEDED_CONFIGURATION)} configuration functions are explicitly superseded."
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--write", action="store_true")
    parser.add_argument(
        "--results", type=pathlib.Path, default=ROOT / "target/validation/reference-tests.jsonl"
    )
    arguments = parser.parse_args()
    document = inventory(ROOT / "target/validation/reference", arguments.results)
    if arguments.write:
        LEDGER.write_text(json.dumps(document, indent=2) + "\n")
    elif document != json.loads(LEDGER.read_text()):
        raise AssertionError("Reference ledger differs from executed cases or pinned sources")
    verify(document)


if __name__ == "__main__":
    main()
