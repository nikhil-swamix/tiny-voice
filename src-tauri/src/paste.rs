use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)] pub struct Target { window: isize, focus: isize, process: u32 }

#[cfg(windows)] mod native {
    use super::Target;
    #[repr(C)] #[derive(Default)] struct Gui { size: u32, flags: u32, active: isize, focus: isize, capture: isize, menu: isize, moving: isize, caret: isize, rect: [i32;4] }
    #[repr(C)] #[derive(Clone, Copy)] struct Key { vk: u16, scan: u16, flags: u32, time: u32, extra: usize }
    #[repr(C)] #[derive(Clone, Copy)] struct Mouse { x: i32, y: i32, data: u32, flags: u32, time: u32, extra: usize }
    #[repr(C)] union Data { key: Key, mouse: Mouse }
    #[repr(C)] struct Input { kind: u32, data: Data }
    #[link(name="user32")] extern "system" {
        fn GetForegroundWindow() -> isize;
        fn GetWindowThreadProcessId(w:isize,p:*mut u32) -> u32;
        fn GetGUIThreadInfo(thread:u32,info:*mut Gui) -> i32;
        fn GetAsyncKeyState(key:i32) -> i16;
        fn SendInput(count:u32,inputs:*const Input,size:i32) -> u32;
    }
    pub fn capture() -> Option<Target> { unsafe {
        let window = GetForegroundWindow(); let mut process = 0;
        let thread = GetWindowThreadProcessId(window,&mut process);
        if window == 0 || thread == 0 || process == 0 || process == std::process::id() { return None; }
        let mut gui = Gui { size:std::mem::size_of::<Gui>() as u32,..Default::default() };
        if GetGUIThreadInfo(thread,&mut gui) == 0 || gui.focus == 0 || gui.active != window || gui.flags & 0x1e != 0 { return None; }
        Some(Target { window,focus:gui.focus,process })
    } }
    fn key(vk:u16,up:bool) -> Input { Input { kind:1,data:Data { key:Key { vk,scan:0,flags:if up { 2 } else { 0 },time:0,extra:0 } } } }
    pub fn send(target:Option<Target>) -> bool { unsafe {
        let Some(target) = target else { return false; };
        if capture().as_ref() != Some(&target) { return false; }
        if [0x10,0x11,0x12,0x5b,0x5c].iter().any(|vk| GetAsyncKeyState(*vk) < 0) { return false; }
        let inputs = [key(0x11,false),key(0x56,false),key(0x56,true),key(0x11,true)];
        // Recheck directly before insertion; never activate or restore a target window.
        if capture().as_ref() != Some(&target) { return false; }
        let sent = SendInput(4,inputs.as_ptr(),std::mem::size_of::<Input>() as i32);
        if sent > 0 && sent < 4 { let release = [key(0x56,true),key(0x11,true)]; SendInput(2,release.as_ptr(),std::mem::size_of::<Input>() as i32); }
        sent == 4
    } }
}
pub fn capture() -> Option<Target> { #[cfg(windows)] { native::capture() } #[cfg(not(windows))] { None } }
pub fn send(target:Option<Target>) -> bool { #[cfg(windows)] { native::send(target) } #[cfg(not(windows))] { let _ = target; false } }
