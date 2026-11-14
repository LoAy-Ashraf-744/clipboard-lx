use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::Path;
use std::ptr;

#[repr(C)]
pub struct Sqlite3 {
    _private: [u8; 0],
}

#[repr(C)]
pub struct Sqlite3Stmt {
    _private: [u8; 0],
}

pub const SQLITE_OK: c_int = 0;
pub const SQLITE_ROW: c_int = 100;
pub const SQLITE_DONE: c_int = 101;

const SQLITE_OPEN_READWRITE: c_int = 0x0000_0002;
const SQLITE_OPEN_CREATE: c_int = 0x0000_0004;
const SQLITE_OPEN_FULLMUTEX: c_int = 0x0001_0000;

fn sqlite_transient() -> *const c_void {
    (-1isize) as *const c_void
}

extern "C" {
    fn sqlite3_open_v2(
        filename: *const c_char,
        pp_db: *mut *mut Sqlite3,
        flags: c_int,
        z_vfs: *const c_char,
    ) -> c_int;
    fn sqlite3_close(db: *mut Sqlite3) -> c_int;
    fn sqlite3_busy_timeout(db: *mut Sqlite3, ms: c_int) -> c_int;
    fn sqlite3_exec(
        db: *mut Sqlite3,
        sql: *const c_char,
        callback: *const c_void,
        arg: *mut c_void,
        errmsg: *mut *mut c_char,
    ) -> c_int;
    fn sqlite3_free(ptr: *mut c_void);
    fn sqlite3_errmsg(db: *mut Sqlite3) -> *const c_char;
    fn sqlite3_changes(db: *mut Sqlite3) -> c_int;
    fn sqlite3_last_insert_rowid(db: *mut Sqlite3) -> i64;

    fn sqlite3_prepare_v2(
        db: *mut Sqlite3,
        z_sql: *const c_char,
        n_byte: c_int,
        pp_stmt: *mut *mut Sqlite3Stmt,
        pz_tail: *mut *const c_char,
    ) -> c_int;
    fn sqlite3_finalize(stmt: *mut Sqlite3Stmt) -> c_int;
    fn sqlite3_step(stmt: *mut Sqlite3Stmt) -> c_int;

    fn sqlite3_bind_int(stmt: *mut Sqlite3Stmt, idx: c_int, val: c_int) -> c_int;
    fn sqlite3_bind_int64(stmt: *mut Sqlite3Stmt, idx: c_int, val: i64) -> c_int;
    fn sqlite3_bind_null(stmt: *mut Sqlite3Stmt, idx: c_int) -> c_int;
    fn sqlite3_bind_text(
        stmt: *mut Sqlite3Stmt,
        idx: c_int,
        val: *const c_char,
        n_bytes: c_int,
        destructor: *const c_void,
    ) -> c_int;
    fn sqlite3_bind_blob(
        stmt: *mut Sqlite3Stmt,
        idx: c_int,
        val: *const c_void,
        n_bytes: c_int,
        destructor: *const c_void,
    ) -> c_int;

    fn sqlite3_column_int(stmt: *mut Sqlite3Stmt, col: c_int) -> c_int;
    fn sqlite3_column_int64(stmt: *mut Sqlite3Stmt, col: c_int) -> i64;
    fn sqlite3_column_text(stmt: *mut Sqlite3Stmt, col: c_int) -> *const u8;
    fn sqlite3_column_blob(stmt: *mut Sqlite3Stmt, col: c_int) -> *const c_void;
    fn sqlite3_column_bytes(stmt: *mut Sqlite3Stmt, col: c_int) -> c_int;
    fn sqlite3_column_type(stmt: *mut Sqlite3Stmt, col: c_int) -> c_int;
}

const SQLITE_NULL: c_int = 5;

pub struct Connection {
    db: *mut Sqlite3,
}

unsafe impl Send for Connection {}

impl Connection {
    pub fn open(path: &Path) -> Result<Self, String> {
        let path_str = path
            .to_str()
            .ok_or_else(|| "Database path is not valid UTF-8".to_string())?;
        let c_path = CString::new(path_str).map_err(|e| e.to_string())?;
        let mut db: *mut Sqlite3 = ptr::null_mut();

        let rc = unsafe {
            sqlite3_open_v2(
                c_path.as_ptr(),
                &mut db,
                SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE | SQLITE_OPEN_FULLMUTEX,
                ptr::null(),
            )
        };

        if rc != SQLITE_OK {
            let msg = if db.is_null() {
                format!("sqlite3_open_v2 failed with code {}", rc)
            } else {
                let err = unsafe { last_error_ptr(db) };
                unsafe { sqlite3_close(db) };
                err
            };
            return Err(msg);
        }

        unsafe {
            sqlite3_busy_timeout(db, 5000);
        }

        Ok(Self { db })
    }

    pub fn execute_batch(&self, sql: &str) -> Result<(), String> {
        let c_sql = CString::new(sql).map_err(|e| e.to_string())?;
        let mut errmsg: *mut c_char = ptr::null_mut();
        let rc = unsafe {
            sqlite3_exec(
                self.db,
                c_sql.as_ptr(),
                ptr::null(),
                ptr::null_mut(),
                &mut errmsg,
            )
        };
        if rc != SQLITE_OK {
            let msg = if !errmsg.is_null() {
                let s = unsafe { CStr::from_ptr(errmsg).to_string_lossy().into_owned() };
                unsafe { sqlite3_free(errmsg as *mut c_void) };
                s
            } else {
                self.last_error()
            };
            return Err(msg);
        }
        Ok(())
    }

    pub fn prepare(&self, sql: &str) -> Result<Statement<'_>, String> {
        let mut stmt: *mut Sqlite3Stmt = ptr::null_mut();
        let bytes = sql.as_bytes();
        let len = c_int::try_from(bytes.len()).map_err(|_| "SQL query too long".to_string())?;
        let rc = unsafe {
            sqlite3_prepare_v2(
                self.db,
                bytes.as_ptr() as *const c_char,
                len,
                &mut stmt,
                ptr::null_mut(),
            )
        };
        if rc != SQLITE_OK {
            return Err(self.last_error());
        }
        Ok(Statement { conn: self, stmt })
    }

    pub fn changes(&self) -> usize {
        unsafe { sqlite3_changes(self.db).max(0) as usize }
    }

    pub fn last_insert_rowid(&self) -> i64 {
        unsafe { sqlite3_last_insert_rowid(self.db) }
    }

    fn last_error(&self) -> String {
        unsafe { last_error_ptr(self.db) }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if !self.db.is_null() {
            unsafe {
                sqlite3_close(self.db);
            }
            self.db = ptr::null_mut();
        }
    }
}

unsafe fn last_error_ptr(db: *mut Sqlite3) -> String {
    let ptr = sqlite3_errmsg(db);
    if ptr.is_null() {
        "Unknown SQLite error".to_string()
    } else {
        CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }
}

pub struct Statement<'conn> {
    conn: &'conn Connection,
    stmt: *mut Sqlite3Stmt,
}

impl<'conn> Statement<'conn> {
    pub fn bind_i32(&mut self, idx: i32, val: i32) -> Result<(), String> {
        let rc = unsafe { sqlite3_bind_int(self.stmt, idx, val) };
        if rc != SQLITE_OK {
            return Err(self.conn.last_error());
        }
        Ok(())
    }

    pub fn bind_i64(&mut self, idx: i32, val: i64) -> Result<(), String> {
        let rc = unsafe { sqlite3_bind_int64(self.stmt, idx, val) };
        if rc != SQLITE_OK {
            return Err(self.conn.last_error());
        }
        Ok(())
    }

    pub fn bind_null(&mut self, idx: i32) -> Result<(), String> {
        let rc = unsafe { sqlite3_bind_null(self.stmt, idx) };
        if rc != SQLITE_OK {
            return Err(self.conn.last_error());
        }
        Ok(())
    }

    pub fn bind_str(&mut self, idx: i32, val: &str) -> Result<(), String> {
        let bytes = val.as_bytes();
        let len = c_int::try_from(bytes.len()).map_err(|_| "Text too long to bind".to_string())?;
        let ptr = if bytes.is_empty() {
            c"".as_ptr()
        } else {
            bytes.as_ptr() as *const c_char
        };
        let rc = unsafe { sqlite3_bind_text(self.stmt, idx, ptr, len, sqlite_transient()) };
        if rc != SQLITE_OK {
            return Err(self.conn.last_error());
        }
        Ok(())
    }

    pub fn bind_blob(&mut self, idx: i32, val: &[u8]) -> Result<(), String> {
        let len = c_int::try_from(val.len()).map_err(|_| "Blob too long to bind".to_string())?;
        let ptr = if val.is_empty() {
            ptr::NonNull::<u8>::dangling().as_ptr() as *const c_void
        } else {
            val.as_ptr() as *const c_void
        };
        let rc = unsafe { sqlite3_bind_blob(self.stmt, idx, ptr, len, sqlite_transient()) };
        if rc != SQLITE_OK {
            return Err(self.conn.last_error());
        }
        Ok(())
    }

    pub fn step(&mut self) -> Result<bool, String> {
        let rc = unsafe { sqlite3_step(self.stmt) };
        match rc {
            SQLITE_ROW => Ok(true),
            SQLITE_DONE => Ok(false),
            _ => Err(self.conn.last_error()),
        }
    }

    pub fn exec_done(&mut self) -> Result<(), String> {
        while self.step()? {}
        Ok(())
    }

    pub fn col_i32(&self, col: i32) -> i32 {
        unsafe { sqlite3_column_int(self.stmt, col) }
    }

    pub fn col_i64(&self, col: i32) -> i64 {
        unsafe { sqlite3_column_int64(self.stmt, col) }
    }

    pub fn col_str(&self, col: i32) -> String {
        unsafe {
            let ptr = sqlite3_column_text(self.stmt, col);
            if ptr.is_null() {
                return String::new();
            }
            let len = sqlite3_column_bytes(self.stmt, col).max(0) as usize;
            let slice = std::slice::from_raw_parts(ptr, len);
            String::from_utf8_lossy(slice).into_owned()
        }
    }

    pub fn col_opt_blob(&self, col: i32) -> Option<Vec<u8>> {
        unsafe {
            if sqlite3_column_type(self.stmt, col) == SQLITE_NULL {
                return None;
            }
            let ptr = sqlite3_column_blob(self.stmt, col);
            let len = sqlite3_column_bytes(self.stmt, col).max(0) as usize;
            if ptr.is_null() || len == 0 {
                return Some(Vec::new());
            }
            let slice = std::slice::from_raw_parts(ptr as *const u8, len);
            Some(slice.to_vec())
        }
    }
}

impl<'conn> Drop for Statement<'conn> {
    fn drop(&mut self) {
        if !self.stmt.is_null() {
            unsafe {
                sqlite3_finalize(self.stmt);
            }
            self.stmt = ptr::null_mut();
        }
    }
}
