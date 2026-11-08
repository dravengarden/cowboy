use cowboy_codeact_runtime::{Host, Sandbox};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};

fn executable(name: &str) -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|path| path.join(name))
        .find(|path| path.is_file())
        .and_then(|path| std::fs::canonicalize(path).ok())
        .unwrap_or_else(|| panic!("required sandbox executable missing: {name}"))
}
struct ParallelHost(tokio::sync::Barrier);
impl Host for ParallelHost {
    async fn call(&self, _op: &str, args: Value, _remaining: Duration) -> Result<Value, String> {
        self.0.wait().await;
        Ok(args)
    }
    fn finish(&self, done: bool, frame: Value) -> Value {
        json!({"done":done,"value":frame["value"]})
    }
    fn call_error(&self, _op: &str, _echo: &str, code: String) -> Value {
        json!({"code":code})
    }
}
#[tokio::test]
async fn independent_calls_reach_the_host_concurrently() {
    let sandbox = Sandbox {
        runtime_directory: None,
        bun_loader: None,
        prlimit: executable("prlimit"),
        bwrap: executable("bwrap"),
        bun: executable("bun"),
    };
    let runner = r#"
for(let id=0;id<4;id++) process.stdout.write(JSON.stringify({id,op:'fixture.read',args:{id}})+'\n');
const values=[];
for await(const line of console) {
  values.push(JSON.parse(line).ok.id);
  if(values.length===4) {
    process.stdout.write(JSON.stringify({done:true,value:values.sort()})+'\n');
    break;
  }
}
"#;
    let result = sandbox
        .execute(
            "",
            false,
            5000,
            runner,
            &ParallelHost(tokio::sync::Barrier::new(4)),
        )
        .await;
    assert_eq!(result, json!({"done":true,"value":[0,1,2,3]}));
}

struct CompletingHost(std::sync::atomic::AtomicBool);
impl Host for CompletingHost {
    async fn call(&self, _op: &str, _args: Value, _remaining: Duration) -> Result<Value, String> {
        tokio::time::sleep(Duration::from_millis(100)).await;
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(Value::Null)
    }
    fn finish(&self, done: bool, frame: Value) -> Value {
        json!({"done":done,"value":frame["value"]})
    }
    fn call_error(&self, _op: &str, _echo: &str, code: String) -> Value {
        json!({"code":code})
    }
}
#[tokio::test]
async fn final_frame_waits_for_admitted_host_cleanup() {
    let sandbox = Sandbox {
        runtime_directory: None,
        bun_loader: None,
        prlimit: executable("prlimit"),
        bwrap: executable("bwrap"),
        bun: executable("bun"),
    };
    let host = CompletingHost(std::sync::atomic::AtomicBool::new(false));
    let runner = r#"process.stdout.write('{"id":1,"op":"fixture.read"}\n');
await new Promise(resolve=>setTimeout(resolve,20));
process.stdout.write('{"done":true,"value":1}\n');"#;
    let result = sandbox.execute("", false, 2000, runner, &host).await;
    assert_eq!(result, json!({"done":true,"value":1}));
    assert!(host.0.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn runtime_directory_exposes_a_relocated_bun_but_not_adjacent_files() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("runtime");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(parent.path().join("private"), "must not be visible").unwrap();
    let bun = root.join("bun");
    std::fs::copy(executable("bun"), &bun).unwrap();
    let sandbox = Sandbox {
        prlimit: executable("prlimit"),
        bwrap: executable("bwrap"),
        bun,
        runtime_directory: Some(root),
        bun_loader: None,
    };
    let runner = format!(
        r#"import {{existsSync}} from 'node:fs';
process.stdout.write(JSON.stringify({{done:true,value:existsSync({})}})+'\n');"#,
        serde_json::to_string(&parent.path().join("private")).unwrap()
    );
    let result = sandbox
        .execute(
            "",
            false,
            3000,
            &runner,
            &ParallelHost(tokio::sync::Barrier::new(1)),
        )
        .await;
    assert_eq!(result, json!({"done":true,"value":false}));
}
