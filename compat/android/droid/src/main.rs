//! droid: run an Android app straight from its APK, with no Android
//! userspace — no init, servicemanager, zygote or system_server.
//!
//! The APK's manifest names the launcher activity; its DEX bytecode runs
//! on a small interpreter whose Java core library and Android API layer
//! are native code (`natives.rs`). The launcher drives the activity
//! lifecycle the way ActivityThread would.
//!
//! Usage: droid <app.apk>

mod axml;
mod dex;
mod natives;
mod vm;
mod zip;

use std::process::ExitCode;
use vm::{Vm, V};

fn load(path: &str) -> Result<(String, String, Vec<dex::Dex>), String> {
    let data = std::fs::read(path).map_err(|e| format!("{}: {}", path, e))?;
    let apk = zip::Zip::parse(data)?;
    let manifest = apk.read("AndroidManifest.xml").ok_or("no AndroidManifest.xml in the APK")?;
    let elements = axml::parse(&manifest)?;
    let (package, activity) = axml::launcher(&elements).ok_or("the manifest declares no launcher activity")?;
    let mut names: Vec<String> =
        apk.names().filter(|n| n.starts_with("classes") && n.ends_with(".dex")).map(String::from).collect();
    names.sort_by_key(|n| (n.len(), n.clone()));
    let mut dexes = Vec::new();
    for n in &names {
        let bytes = apk.read(n).ok_or_else(|| format!("{}: unreadable", n))?;
        dexes.push(dex::Dex::parse(bytes).map_err(|e| format!("{}: {}", n, e))?);
    }
    if dexes.is_empty() {
        return Err("no classes.dex in the APK".into());
    }
    Ok((package, activity, dexes))
}

fn describe(vm: &mut Vm, e: &vm::Ref) -> String {
    natives::jstr(vm, &V::obj(e.clone())).unwrap_or_else(|_| e.class.java_name())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        eprintln!("usage: droid <app.apk>");
        return ExitCode::from(2);
    };
    let (package, activity, dexes) = match load(path) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("droid: {}", e);
            return ExitCode::FAILURE;
        }
    };
    let classes: usize = dexes.iter().map(|d| d.class_defs.len()).sum();
    println!("droid: {}: launching {} ({} classes, {} dex)", package, activity, classes, dexes.len());
    let mut vm = Vm::new(dexes, &package);
    let desc = format!("L{};", activity.replace('.', "/"));

    let start = std::time::Instant::now();
    let result = (|| -> vm::R<()> {
        let class = vm.class(&desc)?;
        if !vm.is_subclass(&class, "Landroid/app/Activity;") {
            return Err(vm.exception("Ljava/lang/ClassCastException;", &format!("{} is not an Activity", activity)));
        }
        vm.init_class(&class)?;
        let act = vm.new_object(&class);
        vm.call(&act, "<init>", "()V", &[])?;
        let finished = |_: &mut Vm| natives::get_field(&act, "mFinished").i() != 0;
        let step = |vm: &mut Vm, name: &str, desc: &str, args: &[V]| -> vm::R<()> {
            vm.call(&act, name, desc, args)?;
            Ok(())
        };
        step(&mut vm, "onCreate", "(Landroid/os/Bundle;)V", &[V::NULL])?;
        if !finished(&mut vm) {
            step(&mut vm, "onStart", "()V", &[])?;
            step(&mut vm, "onPostCreate", "(Landroid/os/Bundle;)V", &[V::NULL])?;
            step(&mut vm, "onResume", "()V", &[])?;
            step(&mut vm, "onPostResume", "()V", &[])?;
            println!("droid: {} resumed in {} ms", activity, start.elapsed().as_millis());
            // No display or input yet: the activity is shut down again.
            step(&mut vm, "onPause", "()V", &[])?;
            step(&mut vm, "onStop", "()V", &[])?;
        }
        step(&mut vm, "onDestroy", "()V", &[])?;
        Ok(())
    })();

    let missing = vm.missing.len();
    match result {
        Ok(()) => {
            println!("droid: {} finished cleanly ({} missing APIs)", activity, missing);
            ExitCode::SUCCESS
        }
        Err(e) => {
            let s = describe(&mut vm, &e);
            println!("droid: uncaught exception: {}", s);
            ExitCode::FAILURE
        }
    }
}
