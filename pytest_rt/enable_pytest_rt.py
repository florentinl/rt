import os

PYTEST_PLUGINS = os.environ.get("PYTEST_PLUGINS")
if PYTEST_PLUGINS is None:
    os.environ["PYTEST_PLUGINS"] = "pytest_rt"
else:
    plugins = PYTEST_PLUGINS.split(",")
    plugins.append("pytest_rt")
    os.environ["PYTEST_PLUGINS"] = ",".join(plugins)
