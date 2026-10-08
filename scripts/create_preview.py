"""Render the product preview from the tested interface sample."""
import subprocess
from pathlib import Path
subprocess.run(["node", "scripts/create-preview.mjs"], cwd=Path(__file__).resolve().parents[1], check=True)
