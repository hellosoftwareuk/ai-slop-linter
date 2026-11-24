"""Fill template.html from data/results.json and data/diffs.json into index.html."""

import json
from pathlib import Path

here = Path(__file__).parent
results = json.loads((here / "data/results.json").read_text())
diffs = json.loads((here / "data/diffs.json").read_text())
headline = next(m for m in results["methods"] if m.get("highlight"))
bench = {
    "methods": results["methods"],
    "languages": results["languages"],
    "testPairs": results["test_pairs"],
}


def signed(value: float) -> str:
    return ("+" if value > 0 else "−") + f"{abs(value):.2f}"


values = {
    "TRAIN_PAIRS": f"{results['train_pairs']:,}",
    "TRAIN_REPOS": f"{results['train_repos']:,}",
    "TEST_PAIRS": f"{results['test_pairs']:,}",
    "CONTROL_PAIRS": f"{results['control_pairs']:,}",
    "HEADLINE_ACC": f"{headline['acc'] * 100:.1f}%",
    "HERO_DELTA": f"{diffs[0]['after'] - diffs[0]['before']:.2f}",
    "CTRL_NAMES": signed(results["controls"]["names"]),
    "CTRL_VERBOSE": signed(results["controls"]["verbose"]),
    "RUN_DATE": results["run_date"],
    "KEPT": str(len(diffs)),
    "CTRL_NAMES_RAW": str(results["controls"]["names"]),
    "CTRL_VERBOSE_RAW": str(results["controls"]["verbose"]),
    "PAIRED_JSON": json.dumps(results["paired"]),
    # "</" cannot appear inside an inline script.
    "DIFFS_JSON": json.dumps(diffs).replace("</", "<\\/"),
    "BENCH_JSON": json.dumps(bench).replace("</", "<\\/"),
}
page = (here / "template.html").read_text()
for key, value in values.items():
    page = page.replace("{{" + key + "}}", value)
assert "{{" not in page, "unfilled placeholder"
(here / "index.html").write_text(page)
print(f"wrote index.html ({len(page) / 1024:.0f} KB)")
