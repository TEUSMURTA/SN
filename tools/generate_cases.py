import json
from pathlib import Path
import sys

output = Path(sys.argv[1])
with output.open("w", encoding="utf-8", newline="\n") as stream:
    for param_1 in range(256):
        case = {"fn": "save_data_name_index", "args": [param_1]}
        stream.write(json.dumps(case, separators=(",", ":")) + "\n")
        for offset in (*range(19), 255):
            case = {"fn": "save_data_name_byte", "args": [param_1, offset]}
            stream.write(json.dumps(case, separators=(",", ":")) + "\n")
