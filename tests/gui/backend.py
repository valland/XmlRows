# Test-only stdin bridge to the actual Rust IPC implementation, without a native window.
from pathlib import Path
import sys
root=Path(__file__).resolve().parents[2]
d=Path(sys.argv[1]);(d/'src').mkdir(parents=True,exist_ok=True)
(d/'Cargo.toml').write_text('[package]\nname="xmlrows-design-backend"\nversion="0.1.0"\nedition="2021"\n[dependencies]\nserde={version="1",features=["derive"]}\nserde_json="1"\nxmlcore={path='+repr(str(root/'crates/xmlcore')).replace("'",'"')+'}\n')
s=(root/'src-tauri/src/lib.rs').read_text().split('#[cfg_attr(mobile')[0]
s=s.replace('use tauri::{Manager, State};','').replace('#[tauri::command]','')
s+='''
pub struct State<'a, T>(&'a T);
impl<'a,T> std::ops::Deref for State<'a,T> {type Target=T;fn deref(&self)->&T{self.0}}
fn main(){
 use std::io::{BufRead,Write};
 let text=std::fs::read_to_string(std::env::args().nth(1).unwrap()).unwrap();
 let state=AppState{doc:Mutex::new(Session{doc:Document::parse(text),path:Some(PathBuf::from(std::env::args().nth(1).unwrap())),rev:0,dirty:false,table_index:Default::default()})};
 for line in std::io::stdin().lock().lines(){
  let v:serde_json::Value=serde_json::from_str(&line.unwrap()).unwrap();
  let a=&v["args"];let id=a["id"].as_u64().unwrap_or(0) as u32;
  let opts:Option<TableOpts>=serde_json::from_value(a["opts"].clone()).ok();
  let result=match v["cmd"].as_str().unwrap(){
   "document_info"=>serde_json::to_value(document_info(State(&state))),
   "document_text"=>serde_json::to_value(document_text(State(&state))),
   "set_text"=>serde_json::to_value(set_text(a["text"].as_str().unwrap().into(),State(&state))),
   "apply_change"=>serde_json::to_value(apply_change(a["from"].as_u64().unwrap() as u32,a["to"].as_u64().unwrap() as u32,a["insert"].as_str().unwrap().into(),State(&state))),
   "table_list"=>serde_json::to_value(table_list(a["offset"].as_u64().unwrap() as usize,a["limit"].as_u64().unwrap() as usize,State(&state))),
   "tree_children"=>serde_json::to_value(tree_children(id,a["offset"].as_u64().unwrap() as usize,a["limit"].as_u64().unwrap() as usize,a["tablesOnly"].as_bool(),State(&state))),
   "node_detail"=>serde_json::to_value(node_detail(id,opts,State(&state)).unwrap()),
   "node_range"=>serde_json::to_value(node_range(id,State(&state))),
   "locate"=>serde_json::to_value(locate(a["offset"].as_u64().unwrap() as u32,State(&state))),
   "sort_group"=>serde_json::to_value(sort_group(id,a["group"].as_u64().unwrap() as usize,a["column"].as_u64().unwrap() as usize,a["ascending"].as_bool().unwrap(),opts,State(&state))),
   "format_document"=>serde_json::to_value(format_document(a["indent"].as_str().unwrap().into(),State(&state))),
   "open_file"=>serde_json::to_value(open_file(a["path"].as_str().unwrap().into(),State(&state)).unwrap()),
   "save_file"=>match save_file(a["path"].as_str().map(String::from),State(&state)) { Ok(info)=>serde_json::to_value(info), Err(e)=>Ok(serde_json::json!({"__error":e})) },
   "export_group"=>serde_json::to_value(export_group(id,a["group"].as_u64().unwrap() as usize,a["sortColumn"].as_u64().map(|x|x as usize),a["ascending"].as_bool().unwrap(),opts,State(&state)).unwrap()),
   _=>Ok(serde_json::Value::Null)
  };
  println!("{}",result.unwrap());std::io::stdout().flush().unwrap();
 }
}
'''
(d/'src/main.rs').write_text(s)
