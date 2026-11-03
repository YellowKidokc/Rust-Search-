"""Interactive collection search without a web server."""
import argparse
import json
from gui import collections, config_path, engine_path, search

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--config')
args = parser.parse_args()
cfg = config_path(args.config)
names = collections(cfg)
if not names:
    raise SystemExit(f'No collections configured in {cfg}. Create/index a collection first.')
for i, name in enumerate(names, 1):
    print(f'{i}. {name}')
number = int(input('Collection number [1]: ') or '1')
if not 1 <= number <= len(names):
    raise SystemExit('Invalid collection number')
while True:
    query = input('Search (blank to exit): ').strip()
    if not query:
        break
    try:
        result = search(engine_path(), cfg, {'collection': names[number-1], 'q': query})
        print(f"{result['matching_passages']} passages in {result['matching_files']} files")
        for hit in result['hits']:
            print(f"\n{hit['path']}\n{hit['location']} | {hit['quote_check']}\n{hit['text']}")
    except Exception as error:
        print(error)
