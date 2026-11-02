import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('gui', Path(__file__).with_name('gui.py'))
gui = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gui)


class SearchBridgeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=os.environ.get('TPSEARCH_TEST_TMP'))
        self.home = Path(self.temp.name)
        self.source = self.home / 'notes'
        self.source.mkdir()
        (self.source / 'test.md').write_text('---\ndomain: Theology\n---\n# Resurrection\nHistorical resurrection testimony.\n', encoding='utf-8')
        self.config = self.home / 'collections.toml'
        self.engine = gui.engine_path()
        subprocess.run([self.engine, 'collection', 'new', 'test', '--config', str(self.config), '--root', str(self.source), '--index-dir', str(self.home / 'index')], check=True, capture_output=True)
        subprocess.run([self.engine, 'index', '-c', 'test', '--config', str(self.config)], check=True, capture_output=True)

    def tearDown(self):
        self.temp.cleanup()

    def test_rust_search_returns_source_and_verified_passage(self):
        self.assertEqual(gui.collections(self.config), ['test'])
        result = gui.search(self.engine, self.config, {'collection': 'test', 'q': 'resurrection', 'type': 'md'})
        self.assertGreater(result['matching_passages'], 0)
        self.assertTrue(any('test.md' in hit['path'] for hit in result['hits']))
        self.assertTrue(all('quote_check' in hit for hit in result['hits']))

    def test_metadata_filter_uses_existing_frontmatter(self):
        result = gui.search(self.engine, self.config, {'collection': 'test', 'q': 'testimony', 'meta': 'domain=Theology'})
        self.assertGreater(result['matching_passages'], 0)
        result = gui.search(self.engine, self.config, {'collection': 'test', 'q': 'testimony', 'meta': 'domain=Finance'})
        self.assertEqual(result['matching_passages'], 0)

    def test_rejects_unregistered_collection_and_invalid_metadata(self):
        with self.assertRaises(ValueError):
            gui.search(self.engine, self.config, {'collection': 'unknown', 'q': 'test'})
        with self.assertRaises(ValueError):
            gui.search(self.engine, self.config, {'collection': 'test', 'q': 'test', 'meta': 'no-equals'})

    def test_query_is_a_literal_argument(self):
        result = gui.search(self.engine, self.config, {'collection': 'test', 'q': 'resurrection & whoami'})
        self.assertIsInstance(result['hits'], list)


if __name__ == '__main__':
    unittest.main()
