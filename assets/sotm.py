import sys

import datetime
import itertools
import subprocess

starting_iteration = int(sys.argv[1]) if len(sys.argv) > 1 else 0
for iteration in itertools.count(starting_iteration):
    print(f'starting attempt {iteration + 1}')
    seed = f'{datetime.datetime.now(datetime.timezone.utc):sotm%Y%m}{iteration}'
    args = [sys.executable, 'test-rs.py', '--release', '--no-emu', '--no-plando', '--cosmetics', '--preset=fenhl_tootr', f'--seed={seed}']
    if iteration > starting_iteration:
        args.append('--no-rebuild')
    if subprocess.run(args).returncode == 0:
        break
    print(f'seed {seed} failed')
