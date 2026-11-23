use std::fs;
use std::path::PathBuf;

/// 从旧数据目录 ~/.ai-tool-manager/ 迁移到 ~/.ai-toolkit/
/// 必须在 Database::new() 之前调用
pub fn migrate_from_old_dir() {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return,
    };

    let old_dir = home.join(".ai-tool-manager");
    let new_dir = home.join(".ai-toolkit");

    // 旧目录不存在，无需迁移
    if !old_dir.exists() {
        return;
    }

    // 新目录已存在且非空，说明用户已用过新版，跳过迁移
    if new_dir.exists() && is_dir_non_empty(&new_dir) {
        return;
    }

    eprintln!("[migration] 检测到旧数据目录 ~/.ai-tool-manager/，开始迁移到 ~/.ai-toolkit/");

    // 1. 创建新目录
    if let Err(e) = fs::create_dir_all(&new_dir) {
        eprintln!("[migration] 创建新目录失败: {}", e);
        return;
    }

    // 2. 迁移数据库文件
    let old_db = old_dir.join("ai-tool-manager.db");
    let new_db = new_dir.join("ai-toolkit.db");
    if old_db.exists() && !new_db.exists() {
        match fs::copy(&old_db, &new_db) {
            Ok(_) => {
                eprintln!("[migration] 数据库文件已复制");
                // 修正数据库内部的旧路径
                fix_db_paths(&new_db);
            }
            Err(e) => {
                eprintln!("[migration] 数据库文件复制失败: {}", e);
            }
        }
    }

    // 3. 移动 detected.json
    let old_detected = old_dir.join("detected.json");
    let new_detected = new_dir.join("detected.json");
    if old_detected.exists() && !new_detected.exists() {
        if let Err(e) = fs::rename(&old_detected, &new_detected) {
            // rename 可能跨文件系统失败，尝试复制+删除
            if let Err(e2) =
                fs::copy(&old_detected, &new_detected).and_then(|_| fs::remove_file(&old_detected))
            {
                eprintln!(
                    "[migration] detected.json 迁移失败: rename={}, copy={}",
                    e, e2
                );
            } else {
                eprintln!("[migration] detected.json 已迁移");
            }
        } else {
            eprintln!("[migration] detected.json 已迁移");
        }
    }

    // 4. 移动 skills/ 目录
    let old_skills = old_dir.join("skills");
    let new_skills = new_dir.join("skills");
    if old_skills.exists() && !new_skills.exists() {
        if let Err(e) = fs::rename(&old_skills, &new_skills) {
            // rename 可能跨文件系统失败，尝试递归复制
            match copy_dir_recursive(&old_skills, &new_skills) {
                Ok(_) => {
                    let _ = fs::remove_dir_all(&old_skills);
                    eprintln!("[migration] skills/ 目录已迁移");
                }
                Err(e2) => {
                    eprintln!(
                        "[migration] skills/ 目录迁移失败: rename={}, copy={}",
                        e, e2
                    );
                }
            }
        } else {
            eprintln!("[migration] skills/ 目录已迁移");
        }
    }

    // 5. 备份旧目录
    backup_old_dir(&old_dir);

    eprintln!("[migration] 迁移完成");
}

/// 从 ~/.ai-toolkit/ 迁移到 ~/.quoly-ai/（产品改名 Quoly AI）。
///
/// 必须在 Database::new() 之前调用。策略：
///   1. 新目录已存在且非空 → 用户已在用新版，跳过（不覆盖）
///   2. 旧目录不存在 → 全新安装，无需迁移
///   3. 否则整目录复制（含 ai-toolkit.db / detected.json / ui-settings.json /
///      crash-trail.log 等全部文件），成功后把旧目录改名为 .bak 保留，
///      不直接删除——里面有几十万个账号记录，出问题要能回退。
///
/// 注意：数据库文件名也从 ai-toolkit.db 改为 quoly-ai.db，复制时要改名。

/// 迁移日志：同时写 stderr 和文件（GUI 程序看不到 stderr，写文件才能排查）
fn mig_log(msg: &str) {
    eprintln!("{msg}");
    if let Some(home) = dirs::home_dir() {
        let p = home.join(".quoly-ai-migration.log");
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true).append(true).open(&p)
        {
            use std::io::Write;
            let _ = writeln!(f, "{} {msg}", chrono::Local::now().format("%H:%M:%S"));
        }
    }
}

pub fn migrate_from_ai_toolkit_dir() {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return,
    };
    let old_dir = home.join(".ai-toolkit");
    let new_dir = home.join(".quoly-ai");

    if !old_dir.exists() {
        return; // 全新安装
    }
    if new_dir.exists() && is_dir_non_empty(&new_dir) {
        return; // 已在用新版
    }

    mig_log(&format!("[migration] 检测到旧数据目录 ~/.ai-toolkit/，开始迁移到 ~/.quoly-ai/"));

    if let Err(e) = fs::create_dir_all(&new_dir) {
        mig_log(&format!("[migration] 创建新目录失败: {e}"));
        return;
    }

    // 逐文件复制（这层目录是平铺的）
    //
    // ⚠️ 数据库必须用 SQLite 在线备份 API 复制，不能直接 fs::copy：
    // 主库文件里可能缺尚未 checkpoint 的 WAL 内容（实测主库 94MB、
    // WAL 里还有 4MB 未合并），直接拷主库会得到一份"少最后几次写入"的库，
    // 表现就是账号列表变空。用 backup API 才能拿到一致快照。
    let entries = match fs::read_dir(&old_dir) {
        Ok(it) => it,
        Err(e) => {
            mig_log(&format!("[migration] 读取旧目录失败: {e}"));
            return;
        }
    };
    let mut copied = 0usize;
    let mut failed = 0usize;
    for entry in entries.flatten() {
        let src = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // 跳过 WAL/SHM：备份 API 已包含其内容，且它们不能单独复制
        if name_str.ends_with("-wal") || name_str.ends_with("-shm") {
            continue;
        }
        // 数据库改名：ai-toolkit.db -> quoly-ai.db
        let is_db = name_str == "ai-toolkit.db";
        let dst = if is_db {
            new_dir.join("quoly-ai.db")
        } else {
            new_dir.join(&name)
        };
        let res = if is_db {
            copy_sqlite_db(&src, &dst)
        } else if src.is_dir() {
            copy_dir_all(&src, &dst)
        } else {
            fs::copy(&src, &dst).map(|_| ())
        };
        match res {
            Ok(_) => copied += 1,
            Err(e) => {
                failed += 1;
                mig_log(&format!("[migration] 复制 {} 失败: {e}", name_str));
            }
        }
    }
    mig_log(&format!("[migration] 复制完成：成功 {copied} 项，失败 {failed} 项"));

    if failed > 0 {
        mig_log(&format!("[migration] 有文件复制失败，为安全起见保留旧目录不动"));
        return;
    }

    // 旧目录改名为 .bak 保留（不删，可回退）
    let backup = home.join(".ai-toolkit.bak-namechange");
    if backup.exists() {
        let _ = fs::remove_dir_all(&backup);
    }
    match fs::rename(&old_dir, &backup) {
        Ok(_) => mig_log("[migration] 旧目录已备份为 ~/.ai-toolkit.bak-namechange"),
        Err(e) => mig_log(&format!(
            "[migration] 旧目录改名失败（不影响新版运行）: {e}"
        )),
    }
}

/// 用 SQLite 在线备份 API 复制数据库（含 WAL 中未落盘的内容）。
///
/// 为什么不能用 `fs::copy`：抓到一个真实的坑——旧库主文件 94MB，但
/// `-wal` 里还有 4MB 未 checkpoint 的写入。直接拷主文件得到的库虽然能打开，
/// 却是「少最后几次写入」的状态（账号表直接变空）。备份 API 会读取一致快照，
/// 把 WAL 内容一并写进目标库。
fn copy_sqlite_db(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    use rusqlite::Connection;
    let src_conn = Connection::open(src).map_err(|e| {
        std::io::Error::other(format!("打开源库失败: {e}"))
    })?;
    let mut dst_conn = Connection::open(dst).map_err(|e| {
        std::io::Error::other(format!("创建目标库失败: {e}"))
    })?;
    let mut backup = rusqlite::backup::Backup::new(&src_conn, &mut dst_conn)
        .map_err(|e| std::io::Error::other(format!("备份初始化失败: {e}")))?;
    // rusqlite 0.32 的 run_to_completion 不允许 pages_per_step <= 0（会 panic
    // "pages_per_step must be positive"，连 -1 也不行）。改用手动 step 循环：
    // 每次拷 256 页，直到返回 Done。
    loop {
        match backup.step(256) {
            Ok(rusqlite::backup::StepResult::Done) => break,
            Ok(_) => continue,
            Err(e) => {
                return Err(std::io::Error::other(format!("备份执行失败: {e}")));
            }
        }
    }
    Ok(())
}

/// 递归复制目录
fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &to)?;
        } else {
            fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// 检查目录是否非空
fn is_dir_non_empty(dir: &PathBuf) -> bool {    match fs::read_dir(dir) {
        Ok(mut entries) => entries.next().is_some(),
        Err(_) => false,
    }
}

/// 修正数据库中存储的旧路径
fn fix_db_paths(db_path: &PathBuf) {
    match rusqlite::Connection::open(db_path) {
        Ok(conn) => {
            // managed_skills.central_path 中包含旧路径
            let _ = conn.execute(
                "UPDATE managed_skills SET central_path = REPLACE(central_path, '.ai-tool-manager', '.ai-toolkit')",
                [],
            );
            // skill_sync_targets.target_path 中包含旧路径
            let _ = conn.execute(
                "UPDATE skill_sync_targets SET target_path = REPLACE(target_path, '.ai-tool-manager', '.ai-toolkit')",
                [],
            );
            eprintln!("[migration] 数据库内部路径已修正");
        }
        Err(e) => {
            eprintln!("[migration] 打开数据库修正路径失败: {}", e);
        }
    }
}

/// 将旧目录重命名为 .ai-tool-manager.bak/，如已存在则加数字后缀
fn backup_old_dir(old_dir: &PathBuf) {
    let home = match old_dir.parent() {
        Some(h) => h,
        None => return,
    };

    let mut bak_dir = home.join(".ai-tool-manager.bak");
    let mut suffix = 1;
    while bak_dir.exists() {
        bak_dir = home.join(format!(".ai-tool-manager.bak.{}", suffix));
        suffix += 1;
    }

    if let Err(e) = fs::rename(old_dir, &bak_dir) {
        eprintln!("[migration] 备份旧目录失败: {}", e);
    } else {
        eprintln!("[migration] 旧目录已备份为 {}", bak_dir.display());
    }
}

/// 递归复制目录
fn copy_dir_recursive(src: &PathBuf, dst: &PathBuf) -> Result<(), std::io::Error> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}
