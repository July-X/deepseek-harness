# Agent Note: `fetch_npm` 解包目标改成 staging 目录，不再把解出来的内容整个删掉

Status: implemented
Archived: 2026-09-01

[English](2026-09-01-desktop-npm-extract-target-dir.md) | 中文

## 问题

每一个从 npm tarball 装的插件——npm 来源插件、以及 git 来源插件在 prepare 构建里用 `pnpm` 拉指定版本时——都在 `validate_plugin` 这一步挂掉，错误是 `不符合 dsh 插件规范：缺少可解析的 package.json`。原有流程（中文注释翻译放代码块外面）：

```rust
extract_tarball(&tgz, &dest.join("package"))   // -> dest/package/{package.json,lib/,...}
let _ = fs::remove_file(&tgz);
let _ = fs::remove_dir_all(dest.join("package"));   // <- deletes the extracted contents
Ok(version)
```

上面三行 `//` 注释里最后那行的中文意思：把刚解出来的内容整个删了。

`fetch_npm` 返回之后，staging 目录 `dest` 里只剩被删掉的 tarball 标记、以及 `git_latest_tag` / `build_git_plugin` 在 git 来源流程里写进去的兄弟文件。`fetch_into_store` 紧接着对一个空目录调 `validate_plugin(&tmp)`，撞上"no package.json"分支，把这条误导性的"缺 manifest"错误报给用户。这个 bug 从最初的社区插件 commit（7ebc6f5a352，2026-08-22）就埋着——每个平台都会撞；这次调查里暴露出来，纯粹是因为上一轮 Windows PATH/tar 修复终于让安装跑到了 `validate_plugin`，空目录才显形。

`extract_tarball` 本身能工作还不够：tarball 里有 `package/` 前缀，靠 `--strip-components=1` 剥掉，插件的 `package.json`、`lib/`、`cordis.patch.yml`……会落到 `tar -C` 指定的那个目录里。把那个目录指向 `dest/package/` 再 `fs::remove_dir_all(dest.join("package"))`，等于自废武功——原本的代码看起来像是在期望"先把内容挪上一层再清理掉临时包装目录"，但那个「挪」的动作从来没发生，清理的永远就是解出来的唯一一份文件。

## 决策

**直接解到 staging 目录，删掉事后清理那行。** 修好的 `fetch_npm` 现在是：

```rust
let tgz = dest.join(".pkg.tgz");
fs::write(&tgz, bytes).map_err(|e| AppError::Io(e.to_string()))?;
extract_tarball(&tgz, dest).map_err(|e| AppError::Plugin(format!("解包失败：{e}（请确认系统存在 tar）")))?;
let _ = fs::remove_file(&tgz);
Ok(version)
```

`dest` 是 `fetch_into_store` 已经通过 `new_staging_dir` 建好的 `tmp-<pid>-<ts>` staging 目录，所以 `extract_tarball` 里的 `fs::create_dir_all(dest)` 这里是空操作，紧接着的 `tar -C dest` 把剥过前缀的内容直接落到 staging 目录根。`validate_plugin(&dest)` 直接读到 `dest/package.json`，`materialize_one` 的 symlink/copy 目标拿到的是真树，`build_git_plugin` / `install_store_deps` 在同一个 staging 目录上跑 `pnpm install` 时也不再面对一个空 workspace。

tarball 清理（`fs::remove_file(&tgz)`）保留——这条一直是对的，只是 diff 的时候跟错误的 package-dir 删除挤在一起被打包带走了。

## 后果

- npm 来源插件的安装在每个平台都成功，不再是只有上一轮 Windows PATH/tar 修复覆盖到的 Windows 才能跑。
- `validate_plugin` 在安装路径上不再需要"缺少可解析的 package.json"这一支；这个分支留下来给真正缺 manifest 的插件作者用，但安装流程不再误触发。
- 完整安装链路终于从 `fetch_npm` → `validate_plugin` → `materialize_one` → `install_store_deps`（link 模式）→ `sync_kernels` → `ensure_wiring` 一路打通。下游这些步骤都不用动——它们原本就假设 staging 目录里就是解出来的树，现在终于名副其实。
- `git` 来源插件之前是工作的，因为 `git clone` 把树直接放在 clone 目标根，不在子目录里；bug 严格局限在 npm 路径上。

## 备选方案

- **把内容向上挪一层再删子目录。** 否决：那种写法要么递归 copy，要么跨已存在目标做目录 rename，最后 staging 目录里装的东西跟「一开始就解到这里」完全一样。直接解过去少一步，也不引入 rename race 窗口。
- **先解到另一个临时目录，再 rename 到 staging 目录。** 否决：staging 目录已经存在、是所有下游步骤（包括 staging-dir 调和表）的权威引用。把另一个目录 rename 到它头上要先确认源为空，并且还要跟 `tmp-<pid>-<ts>` 唯一性检查打架。
- **让 `validate_plugin` 容忍 `package/` 子目录。** 否决：那等于让 bug 继续活着，并且将来任何期望 `validate_plugin` 读 `dir/package.json` 的工具（手工从磁盘读、脚本化安装、materialize 目标的 symlink）都会悄悄选错路径。解包目标才是该严格的地方。