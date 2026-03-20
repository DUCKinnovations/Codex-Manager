# account-register

该目录默认用于放置账号注册脚本与产物。

如果你不想把脚本放在仓库里，也可以把脚本放到仓库外目录（推荐），然后通过环境变量指定。

## 一键导入联动

`账号管理 -> 账号操作 -> 一键导入账号` 会执行以下流程：

1. 先执行 `http_register_v2.py`
2. 再读取 `lanu_results.json`
3. 最后调用现有账号导入接口

默认会在以下位置查找脚本与结果文件：

1. `scripts/account-register/lanu_results.json`
2. `scripts/账号注册/lanu_results.json`
3. `../账号注册/lanu_results.json`（仓库同级目录）

你也可以通过环境变量覆盖路径与解释器：

- `CODEXMANAGER_ACCOUNT_REGISTER_SCRIPT`: 指定 `http_register_v2.py` 绝对路径
- `CODEXMANAGER_ACCOUNT_REGISTER_FILE`: 指定 `lanu_results.json` 绝对路径
- `CODEXMANAGER_ACCOUNT_REGISTER_DIR`: 指定目录（程序会拼接脚本和结果文件名）
- `CODEXMANAGER_ACCOUNT_REGISTER_PYTHON`: 指定 Python 可执行文件（默认依次尝试 `python3`、`python`）

## 脚本使用

把你原目录里的 `config.json`、邮箱 txt 文件和脚本放到本目录后，运行:

```bash
python3 http_register_v2.py
```

运行完成后会生成 `lanu_results.json`，可在界面一键导入。
