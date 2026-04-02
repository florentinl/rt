import os

from uv import find_uv_bin

os.environ["_RT_UV_BIN"] = find_uv_bin()

from rt._rt import Venv, cli_main


def main():
    import sys

    sys.exit(cli_main(sys.argv))
