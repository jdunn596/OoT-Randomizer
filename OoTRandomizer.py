from __future__ import annotations
import sys

import datetime
import logging
import os
import time
from typing import TYPE_CHECKING

from Main import main, from_patch_file, cosmetic_patch, diff_roms
from Utils import check_version, VersionError, local_path

if TYPE_CHECKING:
    from Settings import Settings


def start(settings: Settings, loglevel: int, no_log_file: bool, diff_rom: bool) -> None:
    # set up logger
    logging.basicConfig(format='%(message)s', level=loglevel)
    logger = logging.getLogger('')
    if not no_log_file:
        ts = time.time()
        st = datetime.datetime.fromtimestamp(ts).strftime('%Y-%m-%d %H-%M-%S')
        log_dir = local_path('Logs')
        os.makedirs(log_dir, exist_ok=True)
        log_path = os.path.join(log_dir, '%s.log' % st)
        log_file = logging.FileHandler(log_path)
        log_file.setFormatter(logging.Formatter('[%(asctime)s] %(message)s', datefmt='%H:%M:%S'))
        logger.addHandler(log_file)

    try:
        if diff_rom:
            diff_roms(settings, diff_rom)
        elif settings.cosmetics_only:
            cosmetic_patch(settings)
        elif settings.patch_file != '':
            from_patch_file(settings)
        elif settings.count is not None and settings.count > 1:
            orig_seed = settings.seed
            for i in range(settings.count):
                settings.update_seed(orig_seed + '-' + str(i))
                main(settings)
        else:
            main(settings)
    except Exception as ex:
        logger.exception(ex)
        sys.exit(1)
