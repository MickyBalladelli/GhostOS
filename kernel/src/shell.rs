use core::mem::MaybeUninit;
use syn_shell::{
    Error,
    editor::{EditorAction, Key, LineEditor},
    file_editor::{EditorMode, FileEditor, FileEditorAction},
    filesystem::{
        DeleteMetadata, DirectoryEntry as ShellDirectoryEntry, DirectoryPage,
        DirectoryRemovalMetadata, EntryType, FileMetadata, FileOutput, FilesystemExecutor,
        FilesystemSource, LinkPage,
        Path as ShellPath, PathCompletionPage,
    },
    interpreter::{CommandExecutor, ExecutionToken, Interpreter, InterpreterEvent},
    parser::{CommandCall, CommandRegistry, RouteId, Value},
    render::{OutputFormat, render},
    Text, MAX_LINE_BYTES,
};
use synos_boot_protocol::BootInfo;
use synos_boot_protocol::{BootMethod, MemoryKind};
use synos_power::AcpiPlatform;
use synos_status::{IntoStatus, Status};
use synos_system_model::command::{
    ArgumentKind, ArgumentSpec, CommandSpec, OutputText, OutputValue, StructuredOutput,
};
use crate::capability::{CapabilityObject, CapabilitySpace, Rights};
use crate::monitor::{MonitorState, MonitorView, MAX_LOCKS};
use crate::scheduler::Scheduler;
use crate::task::{AddressSpaceId, CpuId, CpuMask, ThreadId};
use crate::dlm::{DistributedLockManager, NodeFenceTable, DEFAULT_LOCK_CAPACITY, DEFAULT_NODE_FENCE_CAPACITY};
use crate::persistence::PersistentStore;

const HELP_ROUTE: u16 = 1;
const SHOW_SYSTEM_ROUTE: u16 = 2;
const REBOOT_ROUTE: u16 = 3;
const SHUTDOWN_ROUTE: u16 = 4;
const MONITOR_ROUTE: u16 = 5;
const SHOW_PROCESSES_ROUTE: u16 = 6;
const TOP_CPU_ROUTE: u16 = 7;
const SHOW_MEMORY_ROUTE: u16 = 8;
const SHOW_DSM_ROUTE: u16 = 9;
const STOP_JOB_ROUTE: u16 = 10;
const SET_PROCESS_ROUTE: u16 = 11;
const SYNOS_ISOLATE_ROUTE: u16 = 12;
const UPTIME_ROUTE: u16 = 13;
const COMMAND_CAPACITY: usize = 96;
const HISTORY_CAPACITY: usize = 8;
const EDITOR_RENDER_BYTES: usize = 16 * 1024;

// CommandRegistry alone is ~108 KiB. Keep registry/interpreter/executor in BSS via
// MaybeUninit (not const-initialized .data) so they do not inflate kernel.bin, and so
// the shell stack stays small under size-optimized LTO builds.
static mut SHELL_REGISTRY: MaybeUninit<CommandRegistry<COMMAND_CAPACITY>> = MaybeUninit::uninit();
static mut SHELL_INTERPRETER: MaybeUninit<Interpreter> = MaybeUninit::uninit();
static mut SHELL_EXECUTOR: MaybeUninit<KernelExecutor> = MaybeUninit::uninit();
const HELP_CATEGORIES: [&str; 10] = [
    "SHELL",
    "SYSTEM",
    "PROCESS",
    "MEMORY",
    "MONITOR",
    "CONTROL",
    "CLUSTER",
    "FILESYSTEM",
    "FIREWALL",
    "NETWORK",
];

struct ShellLineRender {
    line: Text<MAX_LINE_BYTES>,
    cursor: usize,
}

impl ShellLineRender {
    const fn new() -> Self {
        Self {
            line: Text::empty(),
            cursor: 0,
        }
    }

    fn reset(&mut self) {
        self.line.clear();
        self.cursor = 0;
    }
}

#[derive(Clone, Copy)]
enum EditExit {
    Saved,
    Discarded,
    Cancelled,
}

pub fn run(
    boot_info: &'static BootInfo,
    scheduler: &'static mut Scheduler,
    dlm: &'static DistributedLockManager<DEFAULT_LOCK_CAPACITY>,
    node_fences: &'static NodeFenceTable<DEFAULT_NODE_FENCE_CAPACITY>,
    scheduler_clock: u64,
    acpi: Option<AcpiPlatform>,
) -> ! {
    // Safety: shell::run is entered once from kernel_entry and never re-entered.
    let registry = unsafe {
        let slot = &mut *core::ptr::addr_of_mut!(SHELL_REGISTRY);
        slot.write(CommandRegistry::new());
        slot.assume_init_mut()
    };
    let interpreter = unsafe {
        let slot = &mut *core::ptr::addr_of_mut!(SHELL_INTERPRETER);
        slot.write(Interpreter::new());
        slot.assume_init_mut()
    };
    let help_target = ArgumentSpec::new("COMMAND", ArgumentKind::Text, false, true)
        .expect("valid HELP command argument");
    registry
        .register(
            CommandSpec::new("HELP", &[help_target]).expect("valid HELP command"),
            RouteId::new(HELP_ROUTE).expect("nonzero HELP route"),
        )
        .expect("kernel command registry has capacity");
    register(registry, "SHOW-SYSTEM", SHOW_SYSTEM_ROUTE);
    register(registry, "REBOOT", REBOOT_ROUTE);
    register(registry, "SHUTDOWN", SHUTDOWN_ROUTE);
    register(registry, "MONITOR", MONITOR_ROUTE);
    register(registry, "SHOW-PROCESSES", SHOW_PROCESSES_ROUTE);
    register(registry, "TOP-CPU", TOP_CPU_ROUTE);
    register(registry, "SHOW-MEMORY", SHOW_MEMORY_ROUTE);
    register(registry, "SHOW-DSM", SHOW_DSM_ROUTE);
    register(registry, "UPTIME", UPTIME_ROUTE);
    register_control_commands(registry);
    syn_shell::cluster::register_cluster_commands_without_help(registry)
        .expect("kernel cluster command registry has capacity");
    syn_shell::filesystem::register_filesystem_commands(registry)
        .expect("kernel filesystem command registry has capacity");
    syn_shell::firewall::register_firewall_commands(registry)
        .expect("kernel firewall command registry has capacity");
    syn_shell::network::register_network_commands(registry)
        .expect("kernel network command registry has capacity");

    let mut editor = LineEditor::<HISTORY_CAPACITY>::new();
    // Safety: executor is initialized once before the shell loop and not shared.
    let executor = unsafe {
        let slot = &mut *core::ptr::addr_of_mut!(SHELL_EXECUTOR);
        slot.write(KernelExecutor::new(
            boot_info,
            scheduler,
            dlm,
            node_fences,
            scheduler_clock,
            acpi.is_some(),
        ));
        slot.assume_init_mut()
    };
    let mut keyboard = crate::keyboard::Keyboard::new();
    let mut usb_keyboard = crate::usb_keyboard::UsbKeyboard::new();
    let mut input = VtInput::new();
    let mut ignore_line_feed = false;
    let mut line_render = ShellLineRender::new();

    banner();
    request_terminal_size();
    prompt();

    loop {
        let byte = wait_for_byte(&mut keyboard, &mut usb_keyboard, acpi.as_ref());
        if ignore_line_feed && byte == b'\n' {
            ignore_line_feed = false;
            continue;
        }
        ignore_line_feed = false;

        let key = match byte {
            b'\r' => {
                ignore_line_feed = true;
                Some(Key::Enter)
            }
            _ => input.advance(byte),
        };
        if let Some((columns, rows)) = input.take_resize() {
            crate::console::set_remote_terminal_size(columns, rows)
        }
        let Some(key) = key else { continue };

        match editor.handle(key) {
            Ok(EditorAction::Redraw) => redraw(&editor, &mut line_render),
            Ok(EditorAction::Complete) => {
                if let Err(error) = complete_line(
                    &mut editor,
                    registry,
                    executor,
                    &mut line_render,
                ) {
                    crate::println!();
                    crate::println!("shell completion error: {error:?}");
                    line_render.reset();
                    prompt();
                    redraw(&editor, &mut line_render)
                }
            }
            Ok(EditorAction::Submit(line)) => {
                line_render.reset();
                crate::println!();
                if !line.as_str().trim().is_empty() {
                    execute_line(
                        line.as_str(),
                        registry,
                        interpreter,
                        executor,
                        &mut keyboard,
                        &mut usb_keyboard,
                        acpi.as_ref(),
                    )
                }
                prompt()
            }
            Ok(EditorAction::Cancel) => {
                line_render.reset();
                crate::println!("\x1b[91m^C\x1b[0m");
                prompt()
            }
            Ok(EditorAction::None) => {}
            Err(error) => {
                crate::println!();
                crate::println!("\x1b[91mshell input error:\x1b[0m {error:?}");
                editor.clear();
                line_render.reset();
                prompt()
            }
        }
    }
}

fn register(registry: &mut CommandRegistry<COMMAND_CAPACITY>, name: &str, route: u16) {
    let command = CommandSpec::new(name, &[]).expect("valid kernel command");
    let route = RouteId::new(route).expect("nonzero kernel route");
    registry
        .register(command, route)
        .expect("kernel command registry has capacity")
}

fn register_control_commands(registry: &mut CommandRegistry<COMMAND_CAPACITY>) {
    let job = ArgumentSpec::new("JOB", ArgumentKind::Text, true, true)
        .expect("valid STOP JOB argument");
    let id = ArgumentSpec::new("ID", ArgumentKind::Integer, true, true)
        .expect("valid process id argument");
    let priority = ArgumentSpec::new("PRIORITY", ArgumentKind::Integer, true, false)
        .expect("valid priority argument");
    let cores = ArgumentSpec::new("CORES", ArgumentKind::Text, true, true)
        .expect("valid core list argument");
    registry
        .register(
            CommandSpec::new("STOP", &[job, id]).expect("valid STOP command"),
            RouteId::new(STOP_JOB_ROUTE).expect("valid STOP route"),
        )
        .expect("kernel command registry has capacity");
    registry
        .register(
            CommandSpec::new("SYNOS-ISOLATE", &[cores]).expect("valid isolation command"),
            RouteId::new(SYNOS_ISOLATE_ROUTE).expect("valid isolation route"),
        )
        .expect("kernel command registry has capacity");
    registry
        .register(
            CommandSpec::new("SET-PROCESS", &[id, priority])
                .expect("valid SET PROCESS command"),
            RouteId::new(SET_PROCESS_ROUTE).expect("valid SET PROCESS route"),
        )
        .expect("kernel command registry has capacity")
}

#[inline(never)]
fn execute_line(
    line: &str,
    registry: &CommandRegistry<COMMAND_CAPACITY>,
    interpreter: &mut Interpreter,
    executor: &mut KernelExecutor,
    keyboard: &mut crate::keyboard::Keyboard,
    usb_keyboard: &mut Option<crate::usb_keyboard::UsbKeyboard>,
    acpi: Option<&AcpiPlatform>,
) {
    let program = match registry.parse(line) {
        Ok(program) => program,
        Err(Error::AmbiguousCommand) => {
            crate::println!("shell error: ambiguous command");
            if let Ok(suggestions) = registry.suggestions(line) {
                for command in suggestions.commands() {
                    print_command_suggestion(command.as_str());
                }
            }
            return;
        }
        Err(error) => {
            crate::println!("shell error: {error:?}");
            return;
        }
    };
    if program.background {
        crate::println!("shell error: background jobs not ready");
        return;
    }

    if program.stage_count() == 1
        && program
            .stage_ref(0)
            .is_some_and(|command| !command.json() && is_full_directory_command(command))
    {
        if let Some(command) = program.stage(0) {
            if let Err(status) = executor.print_directory(command, keyboard, usb_keyboard) {
                crate::println!(
                    "command failed: {} (status={:#x})",
                    status_reason(status),
                    status.raw()
                )
            }
        }
        return;
    }

    if program.stage_count() == 1
        && program
            .stage_ref(0)
            .is_some_and(|command| !command.json() && is_full_type_command(command))
    {
        if let Some(command) = program.stage(0) {
            if let Err(status) = executor.print_type(command, keyboard, usb_keyboard) {
                crate::println!(
                    "command failed: {} (status={:#x})",
                    status_reason(status),
                    status.raw()
                )
            }
        }
        return;
    }

    if program.stage_count() == 1
        && program
            .stage_ref(0)
            .is_some_and(|command| !command.json() && is_full_edit_command(command))
    {
        if let Some(command) = program.stage(0) {
            if let Err(status) = executor.edit_file(command, keyboard, usb_keyboard, acpi) {
                crate::println!(
                    "command failed: {} (status={:#x})",
                    status_reason(status),
                    status.raw()
                )
            }
        }
        return;
    }

    let json_output = program
        .stage_ref(0)
        .is_some_and(CommandCall::json);
    let (
        human_help_output,
        human_system_output,
        human_memory_output,
        human_dsm_output,
        human_cpu_output,
        human_monitor_output,
    ) =
        if program.stage_count() == 1 {
            match program.stage_ref(0).map(|stage| stage.route.raw()) {
                Some(HELP_ROUTE) => (true, false, false, false, false, false),
                Some(SHOW_SYSTEM_ROUTE) => (false, true, false, false, false, false),
                Some(SHOW_MEMORY_ROUTE) => (false, false, true, false, false, false),
                Some(SHOW_DSM_ROUTE) => (false, false, false, true, false, false),
                Some(TOP_CPU_ROUTE) => (false, false, false, false, true, false),
                Some(MONITOR_ROUTE) => (false, false, false, false, false, true),
                _ => (false, false, false, false, false, false),
            }
        } else {
            (false, false, false, false, false, false)
        };

    if human_help_output && !json_output {
        if let Some(command) = program.stage_ref(0) {
            if let Some(target) = command.get_text("COMMAND") {
                executor.print_command_help(&registry, target);
            } else {
                executor.print_help(&registry);
            }
        }
        return
    }
    if human_system_output && !json_output {
        executor.print_system();
        return
    }
    if human_memory_output && !json_output {
        executor.print_memory();
        return
    }
    if human_cpu_output && !json_output {
        executor.print_top_cpu();
        return
    }

    match interpreter.start_program(program, executor) {
        Ok(InterpreterEvent::Started) => {}
        Ok(_) => {
            crate::println!("shell error: command did not start");
            return;
        }
        Err(error) => {
            crate::println!("shell error: {error:?}");
            return;
        }
    }

    while interpreter.is_running() {
        match interpreter.poll(executor) {
            Ok(InterpreterEvent::Pending) => crate::arch::halt(),
            Ok(InterpreterEvent::Complete(output)) => {
                if (!human_memory_output && !human_dsm_output && !human_monitor_output)
                    || json_output
                {
                    let format = if json_output {
                        OutputFormat::Json
                    } else {
                        OutputFormat::List
                    };
                    match render(&output, format) {
                        Ok(text) => crate::print!("{}", text.as_str()),
                        Err(error) => crate::println!("shell output error: {error:?}"),
                    }
                }
            }
            Ok(InterpreterEvent::Failed(status)) => {
                crate::println!(
                    "command failed: {} (status={:#x})",
                    status_reason(status),
                    status.raw()
                )
            }
            Ok(InterpreterEvent::Cancelled) => crate::println!("command cancelled"),
            Ok(InterpreterEvent::Submitted(job)) => {
                crate::println!("background job {} submitted", job.raw())
            }
            Ok(InterpreterEvent::Started) => {}
            Err(error) => {
                crate::println!("shell error: {error:?}");
                break;
            }
        }
    }

    if executor.take_reboot_requested() {
        crate::power::reboot(acpi)
    }
    if executor.take_shutdown_requested() {
        crate::power::shutdown(acpi)
    }
}

fn render_file_editor<const CAPACITY: usize>(
    editor: &mut FileEditor<CAPACITY>,
) -> Result<(), Status> {
    let (columns, rows) = crate::console::terminal_size();
    let rendered = editor
        .render::<EDITOR_RENDER_BYTES>(columns, rows)
        .map_err(|_| Status::NO_SPACE)?;
    crate::print!("{}", rendered.as_str());
    Ok(())
}

fn request_terminal_size() {
    crate::print!("\x1b[18t")
}

fn render_file_editor_cursor<const CAPACITY: usize>(
    editor: &mut FileEditor<CAPACITY>,
) -> Result<(), Status> {
    let (columns, rows) = crate::console::terminal_size();
    let rendered = editor
        .render_cursor::<EDITOR_RENDER_BYTES>(columns, rows)
        .map_err(|_| Status::NO_SPACE)?;
    crate::print!("{}", rendered.as_str());
    Ok(())
}

fn render_file_editor_line<const CAPACITY: usize>(
    editor: &mut FileEditor<CAPACITY>,
) -> Result<(), Status> {
    let (columns, rows) = crate::console::terminal_size();
    let rendered = editor
        .render_line_update::<EDITOR_RENDER_BYTES>(columns, rows)
        .map_err(|_| Status::NO_SPACE)?;
    crate::print!("{}", rendered.as_str());
    Ok(())
}

fn is_cursor_only_edit_key(key: Key) -> bool {
    matches!(
        key,
        Key::Left
            | Key::Right
            | Key::Up
            | Key::Down
            | Key::Home
            | Key::End
            | Key::PageUp
            | Key::PageDown
    )
}

fn is_line_only_edit_key<const CAPACITY: usize>(
    editor: &FileEditor<CAPACITY>,
    key: Key,
) -> bool {
    editor.mode() == EditorMode::Insert
        && editor.selected().is_none()
        && matches!(key, Key::Character(_) | Key::Tab)
}

#[cfg(test)]
mod input_tests {
    use super::{
        expand_command, register, register_control_commands, Key, ShellLineRender, VtInput,
        COMMAND_CAPACITY, HISTORY_CAPACITY,
    };
    use syn_shell::{editor::LineEditor, parser::CommandRegistry};

    fn decode(input: &mut VtInput, bytes: &[u8]) -> Key {
        bytes
            .iter()
            .find_map(|byte| input.advance(*byte))
            .expect("VT sequence produces a key")
    }

    #[test]
    fn decodes_up_and_down_without_page_navigation() {
        let mut input = VtInput::new();
        assert_eq!(decode(&mut input, b"\x1b[A"), Key::Up);
        assert_eq!(decode(&mut input, b"\x1b[B"), Key::Down);
        assert_eq!(decode(&mut input, b"\x1bOA"), Key::Up);
        assert_eq!(decode(&mut input, b"\x1bOB"), Key::Down);
        assert_eq!(decode(&mut input, b"\x1b[6~"), Key::PageDown);
    }

    #[test]
    fn decodes_resize_dimensions() {
        let mut input = VtInput::new();
        assert_eq!(decode(&mut input, b"\x1b[8;30;120t"), Key::Resize);
        assert_eq!(input.take_resize(), Some((120, 30)));
    }

    #[test]
    fn tab_completes_abbreviated_show_command_without_repeating_text() {
        let mut registry = CommandRegistry::<COMMAND_CAPACITY>::new();
        register(&mut registry, "SHOW-SYSTEM", 1);
        let mut editor = LineEditor::<HISTORY_CAPACITY>::new();
        editor.replace_line("sho sys").unwrap();
        let mut rendered = ShellLineRender::new();

        assert!(expand_command(&mut editor, &registry, &mut rendered).unwrap());
        assert_eq!(editor.line(), "SHOW SYSTEM");
        assert_eq!(rendered.line.as_str(), "SHOW SYSTEM");
        assert_eq!(rendered.cursor, "SHOW SYSTEM".len());
    }

    #[test]
    fn show_interface_singular_resolves_to_network_command() {
        let mut registry = CommandRegistry::<COMMAND_CAPACITY>::new();
        syn_shell::network::register_network_commands(&mut registry)
            .expect("network commands fit");
        let program = registry
            .parse("SHOW INTERFACE")
            .expect("show interface parses");
        assert_eq!(
            program.stage(0).unwrap().route.raw(),
            syn_shell::network::SHOW_INTERFACES_ROUTE
        );
    }



    #[test]
    fn full_registry_completes_show_int() {
        let registry = full_registry();
        let suggestions = registry.suggestions("show int").expect("suggestions");
        assert!(suggestions.commands().any(|n| n.as_str() == "SHOW-INTERFACES"));
        let suggestions = registry.suggestions("show inter").expect("suggestions");
        assert!(suggestions.commands().any(|n| n.as_str() == "SHOW-INTERFACES"));
        assert_eq!(
            registry.parse("show interfaces").unwrap().stage(0).unwrap().route.raw(),
            syn_shell::network::SHOW_INTERFACES_ROUTE
        );
    }

    fn full_registry() -> CommandRegistry<COMMAND_CAPACITY> {
        let mut registry = CommandRegistry::<COMMAND_CAPACITY>::new();
        let help_target = synos_system_model::command::ArgumentSpec::new(
            "COMMAND",
            synos_system_model::command::ArgumentKind::Text,
            false,
            true,
        )
        .expect("help arg");
        registry
            .register(
                synos_system_model::command::CommandSpec::new("HELP", &[help_target])
                    .expect("help"),
                syn_shell::parser::RouteId::new(1).expect("route"),
            )
            .expect("help register");
        register(&mut registry, "SHOW-SYSTEM", 2);
        register(&mut registry, "REBOOT", 3);
        register(&mut registry, "SHUTDOWN", 4);
        register(&mut registry, "MONITOR", 5);
        register(&mut registry, "SHOW-PROCESSES", 6);
        register(&mut registry, "TOP-CPU", 7);
        register(&mut registry, "SHOW-MEMORY", 8);
        register(&mut registry, "SHOW-DSM", 9);
        register(&mut registry, "UPTIME", 10);
        register_control_commands(&mut registry);
        syn_shell::cluster::register_cluster_commands_without_help(&mut registry)
            .expect("cluster");
        syn_shell::filesystem::register_filesystem_commands(&mut registry)
            .expect("filesystem");
        syn_shell::firewall::register_firewall_commands(&mut registry)
            .expect("firewall");
        syn_shell::network::register_network_commands(&mut registry)
            .expect("network");
        registry
    }

    #[test]
    fn expand_command_accepts_show_int_prefix() {
        let registry = full_registry();
        let mut editor = LineEditor::<HISTORY_CAPACITY>::new();
        editor.replace_line("show int").unwrap();
        let mut rendered = ShellLineRender::new();
        expand_command(&mut editor, &registry, &mut rendered)
            .unwrap_or_else(|error| panic!("expand failed: {error:?}"));
        assert_eq!(editor.line(), "SHOW INTERFACES");
    }

    #[test]
    fn full_registry_parses_show_interfaces() {
        let registry = full_registry();
        let program = registry
            .parse("SHOW INTERFACES")
            .unwrap_or_else(|error| panic!("parse failed: {error:?}"));
        assert_eq!(
            program.stage(0).unwrap().route.raw(),
            syn_shell::network::SHOW_INTERFACES_ROUTE
        );
        let prefix = registry.suggestions("show int").expect("suggestions");
        assert!(prefix.commands().any(|name| name.as_str() == "SHOW-INTERFACES"));
    }


}

fn report_editor_error(status: Status) {
    let (_, rows) = crate::console::terminal_size();
    crate::print!(
        "\x1b[{};1H\x1b[2K\x1b[31mEDIT failed: {} ({:#x})\x1b[0m",
        rows,
        status_reason(status),
        status.raw(),
    );
}

fn prompt_editor_discard() {
    let (_, rows) = crate::console::terminal_size();
    crate::print!(
        "\x1b[{};1H\x1b[2K\x1b[33mUnsaved changes. Discard? [y/N] \x1b[0m",
        rows,
    );
}

fn prompt_editor_conflict() {
    let (_, rows) = crate::console::terminal_size();
    crate::print!(
        "\x1b[{};1H\x1b[2K\x1b[33mFile changed. Save as new version anyway? [y/N] \x1b[0m",
        rows,
    );
}

fn restore_editor_terminal() {
    crate::print!("\x1b[0m\x1b[?25h\x1b[?1049l");
}

fn editor_name<const CAPACITY: usize>(editor: &FileEditor<CAPACITY>) -> &str {
    editor.name()
}

fn print_edit_result<const CAPACITY: usize>(operation: &str, editor: &FileEditor<CAPACITY>) {
    crate::println!(
        "EDIT operation={} status=SUCCESS path={} size={} version={}",
        operation,
        editor_name(editor),
        editor.len(),
        editor.version(),
    );
}

fn prompt() {
    crate::print!("\x1b[1;32mSYNOS\x1b[90m::\x1b[36mROOT\x1b[0m> ")
}

fn parse_cpu_mask(value: &str) -> Result<CpuMask, Status> {
    let mut mask = CpuMask::EMPTY;
    for part in value.split(',') {
        let raw = part.trim().parse::<u8>().map_err(|_| Status::INVALID_ARGUMENT)?;
        let cpu = CpuId::new(raw).ok_or(Status::INVALID_ARGUMENT)?;
        let bit = CpuMask::from_raw(1u64 << cpu.raw());
        if mask.intersects(bit) {
            return Err(Status::INVALID_ARGUMENT)
        }
        mask = mask.union(bit);
    }
    if mask.is_empty() {
        Err(Status::INVALID_ARGUMENT)
    } else {
        Ok(mask)
    }
}

fn is_full_directory_command(command: &CommandCall) -> bool {
    if command.command.as_str().eq_ignore_ascii_case("LS") {
        return true
    }
    command.command.as_str().eq_ignore_ascii_case("DIRECTORY")
        && !matches!(command.get("CREATE"), Some(Value::Boolean(true)))
}

fn is_full_type_command(command: &CommandCall) -> bool {
    command.command.as_str().eq_ignore_ascii_case("TYPE")
}

fn is_full_edit_command(command: &CommandCall) -> bool {
    command.command.as_str().eq_ignore_ascii_case("EDIT")
        || command.command.as_str().eq_ignore_ascii_case("EDT")
}

fn directory_cancelled(
    keyboard: &mut crate::keyboard::Keyboard,
    usb_keyboard: &mut Option<crate::usb_keyboard::UsbKeyboard>,
) -> bool {
    if matches!(keyboard.read_byte(), Some(3)) {
        return true
    }
    if usb_keyboard
        .as_mut()
        .and_then(crate::usb_keyboard::UsbKeyboard::read_byte)
        .is_some_and(|byte| byte == 3)
    {
        return true
    }
    matches!(crate::console::read_byte(), Some(3))
}

struct TypeConsoleOutput<'a> {
    keyboard: &'a mut crate::keyboard::Keyboard,
    usb_keyboard: &'a mut Option<crate::usb_keyboard::UsbKeyboard>,
    binary: bool,
    pending: [u8; 4],
    pending_len: usize,
    cancelled: bool,
}

struct EditBufferOutput<const CAPACITY: usize> {
    bytes: [u8; CAPACITY],
    len: usize,
}

impl<const CAPACITY: usize> EditBufferOutput<CAPACITY> {
    const fn new() -> Self {
        Self {
            bytes: [0; CAPACITY],
            len: 0,
        }
    }
}

impl<const CAPACITY: usize> FileOutput for EditBufferOutput<CAPACITY> {
    fn write(&mut self, bytes: &[u8]) -> Result<(), Status> {
        let end = self.len.checked_add(bytes.len()).ok_or(Status::NO_SPACE)?;
        if end > CAPACITY {
            return Err(Status::NO_SPACE)
        }
        self.bytes[self.len..end].copy_from_slice(bytes);
        self.len = end;
        Ok(())
    }
}

impl<'a> TypeConsoleOutput<'a> {
    fn new(
        keyboard: &'a mut crate::keyboard::Keyboard,
        usb_keyboard: &'a mut Option<crate::usb_keyboard::UsbKeyboard>,
        binary: bool,
    ) -> Self {
        Self {
            keyboard,
            usb_keyboard,
            binary,
            pending: [0; 4],
            pending_len: 0,
            cancelled: false,
        }
    }

    fn finish(&mut self) -> Result<(), Status> {
        if self.cancelled {
            return Err(Status::BUSY)
        }
        if self.pending_len != 0 {
            return Err(Status::INVALID_ARGUMENT)
        }
        Ok(())
    }

    fn print_text(text: &str) {
        for character in text.chars() {
            match character {
                '\n' | '\r' | '\t' => crate::print!("{character}"),
                character if character.is_control() => {
                    crate::print!("\\u{:04x}", character as u32)
                }
                character => crate::print!("{character}"),
            }
        }
    }
}

impl FileOutput for TypeConsoleOutput<'_> {
    fn write(&mut self, bytes: &[u8]) -> Result<(), Status> {
        if directory_cancelled(self.keyboard, self.usb_keyboard) {
            self.cancelled = true;
            return Err(Status::BUSY)
        }
        if self.binary {
            for byte in bytes {
                crate::print!("{:02x}", byte);
            }
            return Ok(())
        }

        let mut combined = [0; 132];
        let combined_len = self.pending_len + bytes.len();
        if combined_len > combined.len() {
            return Err(Status::NO_SPACE)
        }
        combined[..self.pending_len].copy_from_slice(&self.pending[..self.pending_len]);
        combined[self.pending_len..combined_len].copy_from_slice(bytes);
        match core::str::from_utf8(&combined[..combined_len]) {
            Ok(text) => {
                Self::print_text(text);
                self.pending_len = 0;
                Ok(())
            }
            Err(error) if error.error_len().is_some() => Err(Status::INVALID_ARGUMENT),
            Err(error) => {
                let valid = error.valid_up_to();
                let incomplete = combined_len - valid;
                if incomplete > self.pending.len() {
                    return Err(Status::INVALID_ARGUMENT)
                }
                if valid != 0 {
                    let text = core::str::from_utf8(&combined[..valid])
                        .map_err(|_| Status::INVALID_ARGUMENT)?;
                    Self::print_text(text);
                }
                self.pending[..incomplete].copy_from_slice(&combined[valid..combined_len]);
                self.pending_len = incomplete;
                Ok(())
            }
        }
    }
}

fn entry_type_name(entry_type: EntryType) -> &'static str {
    match entry_type {
        EntryType::File => "FILE",
        EntryType::Directory => "DIRECTORY",
        EntryType::Symlink => "SYMLINK",
    }
}

fn complete_line(
    editor: &mut LineEditor<HISTORY_CAPACITY>,
    registry: &CommandRegistry<COMMAND_CAPACITY>,
    executor: &mut KernelExecutor,
    line_render: &mut ShellLineRender,
) -> Result<(), Error> {
    if complete_file(editor, registry, executor, line_render)? {
        return Ok(())
    }

    if expand_command(editor, registry, line_render)? {
        return Ok(())
    }

    let Ok(suggestions) = registry.suggestions(editor.line()) else {
        return Ok(())
    };
    if suggestions.commands().count() > 1 {
        crate::println!();
        for command in suggestions.commands() {
            print_command_suggestion(command.as_str());
        }
        line_render.reset();
        prompt();
        redraw(editor, line_render);
    }
    Ok(())
}

fn complete_file(
    editor: &mut LineEditor<HISTORY_CAPACITY>,
    registry: &CommandRegistry<COMMAND_CAPACITY>,
    executor: &mut KernelExecutor,
    line_render: &mut ShellLineRender,
) -> Result<bool, Error> {
    let line = editor.line();
    if line.trim().is_empty() {
        return Ok(false)
    }
    let Some(command) = registry.unique_suggestion(line).unwrap_or(None) else {
        return Ok(false)
    };
    if !supports_file_completion(command.as_str()) {
        return Ok(false)
    }

    let Some(command_span) = command_span(line) else {
        return Ok(false)
    };
    let (word_start, word_end) = current_word(line, editor.cursor());
    if word_start < command_span.1 {
        return Ok(false)
    }

    let word = &line[word_start..word_end];
    let (directory_input, candidate_prefix, leaf) = match word.rsplit_once('/') {
        Some((prefix, leaf)) => {
            let directory = if prefix.is_empty() { "/" } else { prefix };
            (Some(directory), &word[..word.len() - leaf.len()], leaf)
        }
        None => (None, "", word),
    };
    let directory = match executor.filesystem.session().resolve(directory_input) {
        Ok(directory) if !directory.as_str().contains(';') => directory,
        _ => return Ok(false),
    };

    let mut candidate_prefix_text = Text::<MAX_LINE_BYTES>::empty();
    candidate_prefix_text.push_str(candidate_prefix)?;
    let mut leaf_text = Text::<MAX_LINE_BYTES>::empty();
    leaf_text.push_str(leaf)?;
    complete_file_matches(
        editor,
        executor,
        directory,
        candidate_prefix_text.as_str(),
        leaf_text.as_str(),
        word_start,
        word_end,
        line_render,
    )
}

fn complete_file_matches(
    editor: &mut LineEditor<HISTORY_CAPACITY>,
    executor: &mut KernelExecutor,
    directory: ShellPath,
    candidate_prefix: &str,
    leaf: &str,
    word_start: usize,
    word_end: usize,
    line_render: &mut ShellLineRender,
) -> Result<bool, Error> {
    let mut matches = PathCompletionPage::new();
    if executor
        .filesystem
        .source_mut()
        .complete(directory.as_str(), leaf, &mut matches)
        .is_err()
    {
        return Ok(false)
    }
    let match_count = matches.len();

    if match_count == 0 {
        return Ok(false)
    }

    if match_count == 1 {
        let Some(name) = matches.entries().next() else {
            return Ok(false)
        };
        let mut replacement = Text::<MAX_LINE_BYTES>::empty();
        replacement.push_str(candidate_prefix)?;
        replacement.push_str(name.as_str())?;
        replace_span(editor, word_start, word_end, replacement.as_str())?;
        redraw(editor, line_render);
    } else {
        crate::println!();
        for name in matches.entries() {
            let mut suggestion = Text::<MAX_LINE_BYTES>::empty();
            suggestion.push_str(candidate_prefix)?;
            suggestion.push_str(name.as_str())?;
            crate::println!("  {}", suggestion.as_str());
        }
        line_render.reset();
        prompt();
        redraw(editor, line_render);
    }
    Ok(true)
}

fn expand_command(
    editor: &mut LineEditor<HISTORY_CAPACITY>,
    registry: &CommandRegistry<COMMAND_CAPACITY>,
    line_render: &mut ShellLineRender,
) -> Result<bool, Error> {
    let line = editor.line();
    if line.trim().is_empty() {
        return Ok(false)
    }
    let Some(command) = registry.unique_suggestion(line).unwrap_or(None) else {
        return Ok(false)
    };
    let Some((start, end)) = command_span(line) else {
        return Ok(false)
    };
    let Some(first) = word_span(line, 0) else {
        return Ok(false)
    };
    let is_help = line[first.0..first.1].eq_ignore_ascii_case("HELP");
    if is_help && word_span(line, first.1).is_none() {
        return Ok(false)
    }
    let mut replacement = Text::<MAX_LINE_BYTES>::empty();
    if is_help {
        replacement.push_str("HELP ")?;
    }
    let command_bytes = command.as_str().as_bytes();
    let mut index = 0;
    while index < command_bytes.len() {
        let byte = command_bytes[index];
        replacement.push_char(if byte == b'-' { ' ' } else { byte as char })?;
        index += 1;
    }
    if line[start..end].eq_ignore_ascii_case(replacement.as_str()) {
        return Ok(false)
    }
    replace_span(editor, start, end, replacement.as_str())?;
    redraw(editor, line_render);
    Ok(true)
}

fn supports_file_completion(command: &str) -> bool {
    matches!(
        command,
        "LS"
            | "DIRECTORY"
            | "MKDIR"
            | "CD"
            | "SET-DEFAULT"
            | "TYPE"
            | "CREATE"
            | "DELETE"
            | "SHOW-LINKS"
            | "LINK"
            | "EDIT"
            | "EDT"
    )
}

fn replace_span(
    editor: &mut LineEditor<HISTORY_CAPACITY>,
    start: usize,
    end: usize,
    replacement: &str,
) -> Result<(), Error> {
    let line = editor.line();
    let mut updated = Text::<MAX_LINE_BYTES>::empty();
    updated.push_str(&line[..start])?;
    updated.push_str(replacement)?;
    updated.push_str(&line[end..])?;
    editor.replace_line(updated.as_str())
}

fn command_span(line: &str) -> Option<(usize, usize)> {
    let first = word_span(line, 0)?;
    let first_word = &line[first.0..first.1];
    if first_word.eq_ignore_ascii_case("HELP") {
        let second = word_span(line, first.1)?;
        let second_word = &line[second.0..second.1];
        if matches!(
            second_word,
            value if value.eq_ignore_ascii_case("SHOW")
                || value.eq_ignore_ascii_case("SHO")
                || value.eq_ignore_ascii_case("TOP")
                || value.eq_ignore_ascii_case("SET")
        ) {
            return Some((first.0, word_span(line, second.1).map_or(second.1, |third| third.1)))
        }
        return Some((first.0, second.1))
    }
    if matches!(
        first_word,
        value if value.eq_ignore_ascii_case("SHOW")
            || value.eq_ignore_ascii_case("SHO")
            || value.eq_ignore_ascii_case("TOP")
            || value.eq_ignore_ascii_case("SET")
    ) {
        if let Some(second) = word_span(line, first.1) {
            return Some((first.0, second.1))
        }
    }
    Some(first)
}

fn current_word(line: &str, cursor: usize) -> (usize, usize) {
    let cursor = cursor.min(line.len());
    let bytes = line.as_bytes();
    let mut start = cursor;
    while start > 0 && !bytes[start - 1].is_ascii_whitespace() {
        start -= 1
    }
    let mut end = cursor;
    while end < bytes.len() && !bytes[end].is_ascii_whitespace() {
        end += 1
    }
    (start, end)
}

fn word_span(line: &str, offset: usize) -> Option<(usize, usize)> {
    let bytes = line.as_bytes();
    let mut start = offset.min(bytes.len());
    while start < bytes.len() && bytes[start].is_ascii_whitespace() {
        start += 1
    }
    if start == bytes.len() {
        return None
    }
    let mut end = start;
    while end < bytes.len() && !bytes[end].is_ascii_whitespace() {
        end += 1
    }
    Some((start, end))
}

fn starts_with_ignore_ascii_case(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

fn print_command_suggestion(command: &str) {
    crate::print!("  ");
    print_display_command(command);
    crate::println!();
}

#[inline(never)]
fn print_display_command(command: &str) {
    // Size-optimized LTO miscompiles per-character `print!("{}", char)` into a
    // repeated first character. Transform the token once and print the &str.
    let mut bytes = [0u8; 64];
    let mut len = 0usize;
    for byte in command.as_bytes() {
        if len == bytes.len() {
            break;
        }
        bytes[len] = if *byte == b'-' { b' ' } else { *byte };
        len += 1;
    }
    if let Ok(text) = core::str::from_utf8(&bytes[..len]) {
        crate::print!("{}", text);
    }
}

fn argument_kind(kind: ArgumentKind) -> &'static str {
    match kind {
        ArgumentKind::Boolean => "BOOLEAN",
        ArgumentKind::Integer => "INTEGER",
        ArgumentKind::Text => "TEXT",
    }
}

fn help_category(value: &str) -> Option<&'static str> {
    HELP_CATEGORIES
        .iter()
        .copied()
        .find(|category| category.eq_ignore_ascii_case(value))
}

fn command_category(route: u16) -> &'static str {
    match route {
        HELP_ROUTE => "SHELL",
        SHOW_SYSTEM_ROUTE | REBOOT_ROUTE | SHUTDOWN_ROUTE | UPTIME_ROUTE => "SYSTEM",
        SHOW_PROCESSES_ROUTE | TOP_CPU_ROUTE | STOP_JOB_ROUTE | SET_PROCESS_ROUTE => "PROCESS",
        SHOW_MEMORY_ROUTE | SHOW_DSM_ROUTE => "MEMORY",
        MONITOR_ROUTE => "MONITOR",
        SYNOS_ISOLATE_ROUTE => "CONTROL",
        syn_shell::cluster::SHOW_CLUSTER_ROUTE..=syn_shell::cluster::ABANDON_NODE_ROUTE => {
            "CLUSTER"
        }
        syn_shell::filesystem::DIRECTORY_ROUTE..=syn_shell::filesystem::RMDIR_ROUTE => {
            "FILESYSTEM"
        }
        syn_shell::firewall::SHOW_FIREWALL_ROUTE..=syn_shell::firewall::SET_FIREWALL_ROUTE => {
            "FIREWALL"
        }
        syn_shell::network::SHOW_NETWORK_ROUTE..=syn_shell::network::SET_ROUTE_ROUTE => "NETWORK",
        _ => "SHELL",
    }
}

#[inline(never)]
fn print_command_syntax(spec: &CommandSpec) {
    for argument in spec.arguments().filter(|argument| argument.positional) {
        if argument.required {
            crate::print!(" <");
            print_display_command(argument.name.as_str());
            crate::print!(">");
        } else {
            crate::print!(" [<");
            print_display_command(argument.name.as_str());
            crate::print!(">]");
        }
    }
    for argument in spec.arguments().filter(|argument| !argument.positional) {
        if argument.kind == ArgumentKind::Boolean {
            if argument.required {
                crate::print!(" /");
                print_display_command(argument.name.as_str());
            } else {
                crate::print!(" [/");
                print_display_command(argument.name.as_str());
                crate::print!("]");
            }
        } else if argument.required {
            crate::print!(" /");
            print_display_command(argument.name.as_str());
            crate::print!("=<{}>", argument_kind(argument.kind));
        } else {
            crate::print!(" [/");
            print_display_command(argument.name.as_str());
            crate::print!("=<{}>]", argument_kind(argument.kind));
        }
    }
}

fn redraw<const HISTORY: usize>(
    editor: &LineEditor<HISTORY>,
    rendered: &mut ShellLineRender,
) {
    let current = editor.line();
    crate::print!("\r\x1b[2K");
    prompt();
    crate::print!("{}", current);
    let tail = current.len().saturating_sub(editor.cursor());
    if tail != 0 {
        crate::print!("\x1b[{}D", tail);
    }

    rendered.line.clear();
    let _ = rendered.line.push_str(current);
    rendered.cursor = editor.cursor();
}

fn banner() {
    crate::console::clear();
    crate::println!("\x1b[1;36m{}", r"   _____             ____  _____");
    crate::println!("\x1b[1;96m{}", r"  / ___/__  ______  / __ \/ ___/");
    crate::println!("\x1b[1;94m{}", r"  \__ \/ / / / __ \/ / / /\__ \ ");
    crate::println!("\x1b[1;34m{}", r" ___/ / /_/ / / / / /_/ /___/ /");
    crate::println!("\x1b[1;35m{}", r"/____/\__, /_/ /_/\____//____/");
    crate::println!("\x1b[1;95m{}", r"     /____/                     ");
    crate::println!();
    crate::println!("\x1b[90m  SYNCHRONOUS NETWORK OPERATING SYSTEM // VT100 ONLINE\x1b[0m");
    crate::println!("\x1b[34m  --------------------------------------------------------\x1b[0m");
    crate::println!("\x1b[32m  READY\x1b[90m  TYPE \x1b[37mHELP\x1b[90m FOR COMMANDS\x1b[0m");
    crate::println!()
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum VtInputState {
    Ground,
    Escape,
    Csi,
    Ss3,
}

struct VtInput {
    state: VtInputState,
    parameter: u16,
    modifier: u16,
    third_parameter: u16,
    separators: u8,
    pending: Option<u8>,
    pending_resize: Option<(usize, usize)>,
    utf8: [u8; 4],
    utf8_len: usize,
    utf8_expected: usize,
}

impl VtInput {
    const fn new() -> Self {
        Self {
            state: VtInputState::Ground,
            parameter: 0,
            modifier: 0,
            third_parameter: 0,
            separators: 0,
            pending: None,
            pending_resize: None,
            utf8: [0; 4],
            utf8_len: 0,
            utf8_expected: 0,
        }
    }

    fn advance(&mut self, byte: u8) -> Option<Key> {
        if let Some(pending) = self.pending.take() {
            if let Some(key) = self.advance_now(pending) {
                self.pending = Some(byte);
                return Some(key)
            }
        }
        if self.state == VtInputState::Ground && self.utf8_expected != 0 {
            if byte & 0xc0 != 0x80 || self.utf8_len == self.utf8.len() {
                self.utf8_len = 0;
                self.utf8_expected = 0;
                return None
            }
            self.utf8[self.utf8_len] = byte;
            self.utf8_len += 1;
            if self.utf8_len == self.utf8_expected {
                let character = core::str::from_utf8(&self.utf8[..self.utf8_len])
                    .ok()
                    .and_then(|text| text.chars().next());
                self.utf8_len = 0;
                self.utf8_expected = 0;
                return character.map(Key::Character)
            }
            return None
        }
        self.advance_now(byte)
    }

    fn escape_pending(&self) -> bool {
        self.state == VtInputState::Escape
    }

    fn take_resize(&mut self) -> Option<(usize, usize)> {
        self.pending_resize.take()
    }

    fn flush_escape(&mut self) -> Option<Key> {
        if !self.escape_pending() {
            return None
        }
        self.state = VtInputState::Ground;
        Some(Key::Escape)
    }

    fn advance_now(&mut self, byte: u8) -> Option<Key> {
        match self.state {
            VtInputState::Ground => match byte {
                0x1b => {
                    self.state = VtInputState::Escape;
                    None
                }
                b'\n' | b'\r' => Some(Key::Enter),
                b'\t' => Some(Key::Tab),
                8 | 127 => Some(Key::Backspace),
                19 => Some(Key::Save),
                24 => Some(Key::DiscardExit),
                26 => Some(Key::SaveExit),
                3 => Some(Key::Cancel),
                0xc2..=0xdf => {
                    self.utf8[0] = byte;
                    self.utf8_len = 1;
                    self.utf8_expected = 2;
                    None
                }
                0xe0..=0xef => {
                    self.utf8[0] = byte;
                    self.utf8_len = 1;
                    self.utf8_expected = 3;
                    None
                }
                0xf0..=0xf4 => {
                    self.utf8[0] = byte;
                    self.utf8_len = 1;
                    self.utf8_expected = 4;
                    None
                }
                0x20..=0x7e => Some(Key::Character(byte as char)),
                _ => None,
            },
            VtInputState::Escape => {
                self.parameter = 0;
                self.modifier = 0;
                self.third_parameter = 0;
                self.separators = 0;
                match byte {
                    b'[' => self.state = VtInputState::Csi,
                    b'O' => self.state = VtInputState::Ss3,
                    _ => {
                        self.state = VtInputState::Ground;
                        self.pending = Some(byte);
                        return Some(Key::Escape)
                    }
                }
                None
            }
            VtInputState::Csi => match byte {
                b'0'..=b'9' => {
                    let value = (byte - b'0') as u16;
                    match self.separators {
                        0 => {
                            self.parameter = self.parameter.saturating_mul(10).saturating_add(value)
                        }
                        1 => {
                            self.modifier = self.modifier.saturating_mul(10).saturating_add(value)
                        }
                        _ => {
                            self.third_parameter = self
                                .third_parameter
                                .saturating_mul(10)
                                .saturating_add(value)
                        }
                    }
                    None
                }
                b';' => {
                    self.separators = self.separators.saturating_add(1);
                    None
                }
                _ => {
                    self.state = VtInputState::Ground;
                    self.navigation_key(byte)
                }
            },
            VtInputState::Ss3 => {
                self.state = VtInputState::Ground;
                self.navigation_key(byte)
            }
        }
    }

    fn navigation_key(&mut self, byte: u8) -> Option<Key> {
        let shifted = self.modifier == 2;
        match byte {
            b'A' => Some(if shifted { Key::ShiftUp } else { Key::Up }),
            b'B' => Some(if shifted { Key::ShiftDown } else { Key::Down }),
            b'C' => Some(if shifted { Key::ShiftRight } else { Key::Right }),
            b'D' => Some(if shifted { Key::ShiftLeft } else { Key::Left }),
            b'H' => Some(if shifted { Key::ShiftHome } else { Key::Home }),
            b'F' => Some(if shifted { Key::ShiftEnd } else { Key::End }),
            b't' if self.parameter == 8 && self.separators >= 2 => {
                let rows = self.modifier as usize;
                let columns = self.third_parameter as usize;
                if rows == 0 || columns == 0 {
                    None
                } else {
                    self.pending_resize = Some((columns, rows));
                    Some(Key::Resize)
                }
            }
            b'~' => match self.parameter {
                1 | 7 => Some(if shifted { Key::ShiftHome } else { Key::Home }),
                3 => Some(Key::Delete),
                4 | 8 => Some(if shifted { Key::ShiftEnd } else { Key::End }),
                5 => Some(Key::PageUp),
                6 => Some(Key::PageDown),
                _ => None,
            },
            _ => None,
        }
    }
}

fn read_available_byte(
    keyboard: &mut crate::keyboard::Keyboard,
    usb_keyboard: &mut Option<crate::usb_keyboard::UsbKeyboard>,
) -> Option<u8> {
    keyboard
        .read_byte()
        .or_else(|| {
            usb_keyboard
                .as_mut()
                .and_then(crate::usb_keyboard::UsbKeyboard::read_byte)
        })
        .or_else(crate::console::read_byte)
}

fn wait_for_byte(
    keyboard: &mut crate::keyboard::Keyboard,
    usb_keyboard: &mut Option<crate::usb_keyboard::UsbKeyboard>,
    acpi: Option<&AcpiPlatform>,
) -> u8 {
    loop {
        if let Some(byte) = read_available_byte(keyboard, usb_keyboard) {
            return byte
        }
        if acpi.is_some_and(crate::power::power_button_pressed) {
            crate::power::shutdown(acpi)
        }
        crate::arch::halt()
    }
}

const KERNEL_FILE_CAPACITY: usize = 16;
const KERNEL_FILE_BYTES: usize = 1024;
const KERNEL_PERSISTENCE_BYTES: usize = 32 * 1024;
const PERSISTENCE_MAGIC: &[u8; 8] = b"SYNFS001";
const PERSISTENCE_VERSION: u32 = 1;

#[derive(Clone, Copy)]
struct KernelFile {
    path: ShellPath,
    file_type: EntryType,
    version: u32,
    link_count: u32,
    object_id: u64,
    is_link: bool,
    deleted: bool,
}

#[derive(Clone, Copy)]
struct KernelObject {
    bytes: [u8; KERNEL_FILE_BYTES],
    length: usize,
}

struct KernelFilesystem {
    files: [Option<KernelFile>; KERNEL_FILE_CAPACITY],
    objects: [Option<KernelObject>; KERNEL_FILE_CAPACITY],
    next_object_id: u64,
    persistent: PersistentStore,
    scratch: [u8; KERNEL_PERSISTENCE_BYTES],
}

impl KernelFilesystem {
    fn new() -> Self {
        let mut filesystem = Self {
            files: [None; KERNEL_FILE_CAPACITY],
            objects: [None; KERNEL_FILE_CAPACITY],
            next_object_id: 1,
            persistent: PersistentStore::new(),
            scratch: [0; KERNEL_PERSISTENCE_BYTES],
        };
        for path in ["/packages", "/logs", "/data", "/tmp"] {
            let _ = filesystem.insert(path, EntryType::Directory);
        }
        filesystem.load_persistent();
        filesystem
    }

    fn load_persistent(&mut self) {
        let Some(length) = self.persistent.load(&mut self.scratch) else {
            return
        };
        let Some((files, objects, next_object_id)) = decode_filesystem(&self.scratch[..length]) else {
            return
        };
        self.files = files;
        self.objects = objects;
        self.next_object_id = next_object_id;
    }

    fn persist(&mut self) {
        let Some(length) = encode_filesystem(
            &self.files,
            &self.objects,
            self.next_object_id,
            &mut self.scratch,
        ) else {
            return
        };
        self.persistent.save(&self.scratch[..length]);
    }

    fn find(&self, path: &str) -> Option<KernelFile> {
        self.files
            .iter()
            .flatten()
            .filter(|file| file.path.as_str() == path && !file.deleted)
            .max_by_key(|file| file.version)
            .copied()
    }

    fn find_version(&self, path: &str, version: u32) -> Option<KernelFile> {
        self.files
            .iter()
            .flatten()
            .find(|file| file.path.as_str() == path && file.version == version && !file.deleted)
            .copied()
    }

    fn insert(&mut self, path: &str, file_type: EntryType) -> Result<KernelFile, Status> {
        if self.find(path).is_some() {
            return Err(Status::ALREADY_EXISTS)
        }
        self.insert_version(path, file_type, 1)
    }

    fn insert_version(
        &mut self,
        path: &str,
        file_type: EntryType,
        version: u32,
    ) -> Result<KernelFile, Status> {
        let slot = self
            .files
            .iter_mut()
            .find(|file| file.is_none())
            .ok_or(Status::NO_SPACE)?;
        let object_id = self.next_object_id;
        let object = self
            .objects
            .iter_mut()
            .find(|object| object.is_none())
            .ok_or(Status::NO_SPACE)?;
        *object = Some(KernelObject {
            bytes: [0; KERNEL_FILE_BYTES],
            length: 0,
        });
        let file = KernelFile {
            path: ShellPath::new(path)?,
            file_type,
            version,
            link_count: 1,
            object_id,
            is_link: false,
            deleted: false,
        };
        self.next_object_id = self.next_object_id.saturating_add(1);
        *slot = Some(file);
        Ok(file)
    }

    fn parent(path: &str) -> &str {
        let path = path.trim_end_matches('/');
        let Some(separator) = path.rfind('/') else { return "/" };
        let parent = path[..separator].trim_end_matches('/');
        if parent.is_empty() { "/" } else { parent }
    }

    fn ensure_parent_directories(&mut self, path: &str) -> Result<(), Status> {
        if self.directory_exists(path)? {
            return Ok(())
        }
        let parent = Self::parent(path);
        if parent != "/" {
            self.ensure_parent_directories(parent)?;
        }
        self.insert(path, EntryType::Directory).map(|_| ())
    }

    fn metadata(&self, file: KernelFile) -> FileMetadata {
        let size = self.object(file).map_or(0, |object| object.length);
        FileMetadata {
            path: file.path,
            file_type: file.file_type,
            size: size as u64,
            version: file.version,
            link_count: self.link_count(file.object_id),
            is_link: file.is_link,
        }
    }

    fn object(&self, file: KernelFile) -> Option<KernelObject> {
        self.objects
            .get(file.object_id.checked_sub(1)? as usize)
            .copied()
            .flatten()
    }

    fn link_count(&self, object_id: u64) -> u32 {
        self.files
            .iter()
            .flatten()
            .filter(|file| {
                file.object_id == object_id
                    && self
                        .find(file.path.as_str())
                        .is_some_and(|latest| latest.version == file.version)
            })
            .count() as u32
    }
}

struct PersistenceWriter<'a> {
    bytes: &'a mut [u8],
    cursor: usize,
}

impl<'a> PersistenceWriter<'a> {
    fn new(bytes: &'a mut [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn write(&mut self, bytes: &[u8]) -> Option<()> {
        let end = self.cursor.checked_add(bytes.len())?;
        let target = self.bytes.get_mut(self.cursor..end)?;
        target.copy_from_slice(bytes);
        self.cursor = end;
        Some(())
    }

    fn u8(&mut self, value: u8) -> Option<()> {
        self.write(&[value])
    }

    fn u16(&mut self, value: u16) -> Option<()> {
        self.write(&value.to_le_bytes())
    }

    fn u32(&mut self, value: u32) -> Option<()> {
        self.write(&value.to_le_bytes())
    }

    fn u64(&mut self, value: u64) -> Option<()> {
        self.write(&value.to_le_bytes())
    }
}

struct PersistenceReader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> PersistenceReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn read(&mut self, length: usize) -> Option<&'a [u8]> {
        let end = self.cursor.checked_add(length)?;
        let bytes = self.bytes.get(self.cursor..end)?;
        self.cursor = end;
        Some(bytes)
    }

    fn u8(&mut self) -> Option<u8> {
        self.read(1).map(|bytes| bytes[0])
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.read(2)?.try_into().ok()?))
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.read(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.read(8)?.try_into().ok()?))
    }
}

fn encode_filesystem(
    files: &[Option<KernelFile>; KERNEL_FILE_CAPACITY],
    objects: &[Option<KernelObject>; KERNEL_FILE_CAPACITY],
    next_object_id: u64,
    bytes: &mut [u8],
) -> Option<usize> {
    let mut writer = PersistenceWriter::new(bytes);
    writer.write(PERSISTENCE_MAGIC)?;
    writer.u32(PERSISTENCE_VERSION)?;
    writer.u64(next_object_id)?;
    for file in *files {
        let Some(file) = file else {
            writer.u8(0)?;
            continue
        };
        writer.u8(1)?;
        let path = file.path.as_str().as_bytes();
        writer.u16(u16::try_from(path.len()).ok()?)?;
        writer.write(path)?;
        writer.u8(entry_type_code(file.file_type))?;
        writer.u32(file.version)?;
        writer.u32(file.link_count)?;
        writer.u64(file.object_id)?;
        writer.u8(file.is_link as u8)?;
        writer.u8(file.deleted as u8)?;
    }
    for object in *objects {
        let Some(object) = object else {
            writer.u8(0)?;
            continue
        };
        writer.u8(1)?;
        writer.u16(u16::try_from(object.length).ok()?)?;
        writer.write(&object.bytes[..object.length])?;
    }
    Some(writer.cursor)
}

fn decode_filesystem(
    bytes: &[u8],
) -> Option<(
    [Option<KernelFile>; KERNEL_FILE_CAPACITY],
    [Option<KernelObject>; KERNEL_FILE_CAPACITY],
    u64,
)> {
    let mut reader = PersistenceReader::new(bytes);
    if reader.read(PERSISTENCE_MAGIC.len())? != PERSISTENCE_MAGIC
        || reader.u32()? != PERSISTENCE_VERSION
    {
        return None
    }
    let next_object_id = reader.u64()?;
    if next_object_id == 0 {
        return None
    }
    let mut files = [None; KERNEL_FILE_CAPACITY];
    for slot in &mut files {
        if reader.u8()? == 0 {
            continue
        }
        let path_length = reader.u16()? as usize;
        let path = core::str::from_utf8(reader.read(path_length)?).ok()?;
        let file_type = decode_entry_type(reader.u8()?)?;
        let version = reader.u32()?;
        let link_count = reader.u32()?;
        let object_id = reader.u64()?;
        if object_id == 0 || object_id as usize > KERNEL_FILE_CAPACITY {
            return None
        }
        *slot = Some(KernelFile {
            path: ShellPath::new(path).ok()?,
            file_type,
            version,
            link_count,
            object_id,
            is_link: reader.u8()? != 0,
            deleted: reader.u8()? != 0,
        });
    }
    let mut objects = [None; KERNEL_FILE_CAPACITY];
    for slot in &mut objects {
        if reader.u8()? == 0 {
            continue
        }
        let length = reader.u16()? as usize;
        if length > KERNEL_FILE_BYTES {
            return None
        }
        let mut object = KernelObject {
            bytes: [0; KERNEL_FILE_BYTES],
            length,
        };
        object.bytes[..length].copy_from_slice(reader.read(length)?);
        *slot = Some(object);
    }
    Some((files, objects, next_object_id))
}

fn entry_type_code(file_type: EntryType) -> u8 {
    match file_type {
        EntryType::File => 1,
        EntryType::Directory => 2,
        EntryType::Symlink => 3,
    }
}

fn decode_entry_type(code: u8) -> Option<EntryType> {
    match code {
        1 => Some(EntryType::File),
        2 => Some(EntryType::Directory),
        3 => Some(EntryType::Symlink),
        _ => None,
    }
}

impl FilesystemSource for KernelFilesystem {
    fn directory_exists(&mut self, path: &str) -> Result<bool, Status> {
        Ok(path == "/"
            || self
                .find(path)
                .is_some_and(|file| file.file_type == EntryType::Directory))
    }

    fn version_exists(&mut self, path: &str, version: u32) -> Result<bool, Status> {
        Ok(self.find_version(path, version).is_some())
    }

    fn complete(
        &mut self,
        directory: &str,
        prefix: &str,
        output: &mut PathCompletionPage,
    ) -> Result<(), Status> {
        if directory != "/"
            && self
                .find(directory)
                .is_some_and(|file| file.file_type != EntryType::Directory)
        {
            return Err(Status::NOT_DIRECTORY)
        }
        if !self.directory_exists(directory)? {
            return Err(Status::NOT_FOUND)
        }
        output.clear();
        for file in self.files.iter().flatten() {
            if file.deleted || Self::parent(file.path.as_str()) != directory {
                continue
            }
            let name = file.path.as_str().rsplit('/').next().unwrap_or("");
            if !starts_with_ignore_ascii_case(name, prefix) {
                continue
            }
            output.push(ShellPath::new(name)?)?;
            if output.len() == syn_shell::filesystem::MAX_PATH_COMPLETION_MATCHES {
                break
            }
        }
        Ok(())
    }

    fn list(
        &mut self,
        path: &str,
        continuation: Option<u32>,
        output: &mut syn_shell::filesystem::DirectoryPage,
    ) -> Result<(), Status> {
        if path != "/"
            && self
                .find(path)
                .is_some_and(|file| file.file_type != EntryType::Directory)
        {
            return Err(Status::NOT_DIRECTORY)
        }
        if !self.directory_exists(path)? {
            return Err(Status::NOT_FOUND)
        }
        output.clear();
        let start = continuation.unwrap_or(0) as usize;
        let mut index = 0usize;
        for slot in self.files.iter() {
            let Some(file) = slot else {
                continue
            };
            if file.deleted {
                continue
            }
            let current = index;
            index += 1;
            if current < start {
                continue
            }
            if Self::parent(file.path.as_str()) != path {
                continue
            }
            if output.len() == syn_shell::filesystem::MAX_DIRECTORY_PAGE_ENTRIES {
                output.next = Some(current as u32);
                break
            }
            let name = file.path.as_str().rsplit('/').next().unwrap_or("");
            output.push(ShellDirectoryEntry {
                name: ShellPath::new(name)?,
                file_type: file.file_type,
                size: self.object(*file).map_or(0, |object| object.length) as u64,
                version: file.version,
                link_count: self.link_count(file.object_id),
                is_link: file.is_link,
            })?;
        }
        Ok(())
    }

    fn create_directory(
        &mut self,
        path: &str,
        recursive: bool,
    ) -> Result<FileMetadata, Status> {
        if path == "/" {
            return Err(Status::INVALID_PATH)
        }
        if self.find(path).is_some() {
            return Err(Status::ALREADY_EXISTS)
        }
        if recursive {
            self.ensure_parent_directories(Self::parent(path))?;
        } else if !self.directory_exists(Self::parent(path))? {
            return Err(Status::NOT_FOUND)
        }
        self.insert(path, EntryType::Directory)
            .map(|file| self.metadata(file))
    }

    fn create_file(&mut self, path: &str) -> Result<FileMetadata, Status> {
        if path == "/" {
            return Err(Status::INVALID_PATH)
        }
        let parent = Self::parent(path);
        if self
            .find(parent)
            .is_some_and(|file| file.file_type != EntryType::Directory)
        {
            return Err(Status::NOT_DIRECTORY)
        }
        if !self.directory_exists(parent)? {
            return Err(Status::NOT_FOUND)
        }
        if self.find(path).is_some() {
            return Err(Status::ALREADY_EXISTS)
        }
        self.insert_version(path, EntryType::File, 1)
            .map(|file| self.metadata(file))
    }

    fn remove_directory(&mut self, path: &str) -> Result<DirectoryRemovalMetadata, Status> {
        if path == "/" {
            return Err(Status::INVALID_PATH)
        }
        let selected = self.find(path).ok_or(Status::NOT_FOUND)?;
        if selected.file_type != EntryType::Directory {
            return Err(Status::NOT_DIRECTORY)
        }
        if self
            .files
            .iter()
            .flatten()
            .any(|file| !file.deleted && Self::parent(file.path.as_str()) == path)
        {
            return Err(Status::DIRECTORY_NOT_EMPTY)
        }
        let slot = self
            .files
            .iter_mut()
            .find(|file| {
                file.is_some_and(|file| {
                    file.path == selected.path
                        && file.version == selected.version
                        && !file.deleted
                })
            })
            .ok_or(Status::CORRUPT)?;
        slot.as_mut().ok_or(Status::CORRUPT)?.deleted = true;
        Ok(DirectoryRemovalMetadata {
            directory: self.metadata(selected),
            removal_generation: selected.version as u64,
            storage_reclamation_pending: false,
        })
    }

    fn delete(&mut self, path: &str) -> Result<DeleteMetadata, Status> {
        let (path, version) = syn_shell::filesystem::split_version_selector(path)?;
        if path == "/" {
            return Err(Status::INVALID_PATH)
        }
        let selected = self
            .files
            .iter()
            .flatten()
            .filter(|file| {
                file.path.as_str() == path
                    && !file.deleted
                    && version.map_or(true, |version| version == 0 || file.version == version)
            })
            .max_by_key(|file| file.version)
            .copied()
            .ok_or(Status::NOT_FOUND)?;
        if selected.file_type == EntryType::Directory {
            return Err(Status::NOT_DIRECTORY)
        }
        let shared_before = self.link_count(selected.object_id);
        let slot = self
            .files
            .iter_mut()
            .find(|file| {
                file.is_some_and(|file| {
                    file.path == selected.path
                        && file.version == selected.version
                        && !file.deleted
                })
            })
            .ok_or(Status::CORRUPT)?;
        slot.as_mut().ok_or(Status::CORRUPT)?.deleted = true;
        let mut deleted = selected;
        deleted.is_link |= shared_before > 1;
        deleted.link_count = self.link_count(selected.object_id);
        Ok(DeleteMetadata {
            file: self.metadata(deleted),
            shared_data_reachable: deleted.link_count != 0,
        })
    }

    fn link(&mut self, source: &str, target: &str) -> Result<FileMetadata, Status> {
        let (source_path, version) = syn_shell::filesystem::split_version_selector(source)?;
        if syn_shell::filesystem::split_version_selector(target)?.1.is_some()
            || target == "/"
            || self.find(target).is_some()
        {
            return Err(if self.find(target).is_some() {
                Status::ALREADY_EXISTS
            } else {
                Status::INVALID_ARGUMENT
            })
        }
        let source = match version {
            None | Some(0) => self.find(source_path),
            Some(version) => self.find_version(source_path, version),
        }
        .ok_or(Status::NOT_FOUND)?;
        if source.file_type != EntryType::File {
            return Err(Status::NOT_DIRECTORY)
        }
        if !self.directory_exists(Self::parent(target))? {
            return Err(Status::NOT_FOUND)
        }
        let slot = self
            .files
            .iter_mut()
            .find(|file| file.is_none())
            .ok_or(Status::NO_SPACE)?;
        let mut linked = source;
        linked.path = ShellPath::new(target)?;
        linked.version = 1;
        linked.link_count = 1;
        linked.is_link = true;
        *slot = Some(linked);
        Ok(self.metadata(linked))
    }

    fn list_links(&mut self, path: &str, output: &mut LinkPage) -> Result<FileMetadata, Status> {
        let (path, version) = syn_shell::filesystem::split_version_selector(path)?;
        let selected = match version {
            None | Some(0) => self.find(path),
            Some(version) => self.find_version(path, version),
        }
        .ok_or(Status::NOT_FOUND)?;
        output.clear();
        for file in self.files.iter().flatten() {
            if file.deleted {
                continue
            }
            if file.object_id != selected.object_id
                || !self
                    .find(file.path.as_str())
                    .is_some_and(|latest| latest.version == file.version)
            {
                continue
            }
            output.push(file.path)?;
        }
        Ok(self.metadata(selected))
    }

    fn type_file(
        &mut self,
        path: &str,
        _binary: bool,
        output: &mut dyn FileOutput,
    ) -> Result<FileMetadata, Status> {
        let (path, version) = syn_shell::filesystem::split_version_selector(path)?;
        if path == "/" {
            return Err(Status::INVALID_PATH)
        }
        let file = match version {
            None | Some(0) => self.find(path),
            Some(version) => self.find_version(path, version),
        }
        .ok_or(Status::NOT_FOUND)?;
        if file.file_type != EntryType::File {
            return Err(Status::NOT_DIRECTORY)
        }
        if version.is_some_and(|version| version != 0 && version != file.version) {
            return Err(Status::NOT_FOUND)
        }
        let object = self.object(file).ok_or(Status::CORRUPT)?;
        for chunk in object.bytes[..object.length].chunks(128) {
            output.write(chunk)?;
        }
        Ok(self.metadata(file))
    }

    fn save_file(&mut self, path: &str, contents: &[u8]) -> Result<FileMetadata, Status> {
        let (path, version) = syn_shell::filesystem::split_version_selector(path)?;
        if path == "/" || contents.len() > KERNEL_FILE_BYTES {
            return if path == "/" {
                Err(Status::INVALID_PATH)
            } else {
                Err(Status::NO_SPACE)
            }
        }
        let selected = match version {
            None | Some(0) => self.find(path),
            Some(version) => self.find_version(path, version),
        }
        .ok_or(Status::NOT_FOUND)?;
        if selected.file_type != EntryType::File {
            return Err(Status::NOT_DIRECTORY)
        }
        if version.is_some_and(|version| version != 0 && version != selected.version) {
            return Err(Status::NOT_FOUND)
        }
        let next_version = self
            .files
            .iter()
            .flatten()
            .filter(|file| file.path.as_str() == path && !file.deleted)
            .map(|file| file.version)
            .max()
            .ok_or(Status::CORRUPT)?
            .checked_add(1)
            .ok_or(Status::CORRUPT)?;
        let saved = self.insert_version(path, EntryType::File, next_version)?;
        let object = self
            .objects
            .get_mut(saved.object_id.checked_sub(1).ok_or(Status::CORRUPT)? as usize)
            .and_then(Option::as_mut)
            .ok_or(Status::CORRUPT)?;
        object.bytes[..contents.len()].copy_from_slice(contents);
        object.length = contents.len();
        Ok(self.metadata(saved))
    }

    fn save_file_if_version(
        &mut self,
        path: &str,
        expected_version: u32,
        contents: &[u8],
    ) -> Result<FileMetadata, Status> {
        let (source_path, version) = syn_shell::filesystem::split_version_selector(path)?;
        let selected = match version {
            None | Some(0) => self.find(source_path),
            Some(version) => self.find_version(source_path, version),
        }
        .ok_or(Status::NOT_FOUND)?;
        if selected.version != expected_version {
            return Err(Status::CONFLICT)
        }
        if version.is_none() || version == Some(0) {
            let latest = self.find(source_path).ok_or(Status::NOT_FOUND)?;
            if latest.version != expected_version {
                return Err(Status::CONFLICT)
            }
        }
        self.save_file(path, contents)
    }

    fn save_file_force(
        &mut self,
        path: &str,
        contents: &[u8],
    ) -> Result<FileMetadata, Status> {
        self.save_file(path, contents)
    }
}

struct KernelNetwork {
    view: syn_shell::network::NetworkView,
    devices: [Option<synos_legacy_pc_drivers::EthernetRuntime>;
        syn_shell::network::MAX_NETWORK_OUTPUT_ROWS - 1],
}

impl KernelNetwork {
    fn new(boot_info: &'static BootInfo) -> Self {
        use syn_shell::network::{
            InterfaceAddressMode, MAX_NETWORK_OUTPUT_ROWS, NetworkInterfaceView, NetworkText,
            NetworkView,
        };

        let text = |value: &str| NetworkText::new(value).expect("network text fits");
        let mut interfaces = [None; MAX_NETWORK_OUTPUT_ROWS];
        interfaces[0] = Some(NetworkInterfaceView {
            name: text("lo"),
            address: text("127.0.0.1"),
            prefix_len: None,
            mac: None,
            gateway: None,
            mtu: 65_535,
            enabled: true,
            link_up: true,
            rx_queue: None,
            tx_queue: None,
            mode: InterfaceAddressMode::Static,
            dhcp: None,
        });
        let devices = discover_runtime_network(boot_info);
        let mut network = Self {
            view: NetworkView {
                generation: 1,
                hostname: Some(text("synos")),
                interface_count: 1,
                route_count: 0,
                interfaces,
                routes: [None; MAX_NETWORK_OUTPUT_ROWS],
                link_events: [None; syn_shell::network::MAX_NETWORK_LINK_EVENTS],
                next_interface: None,
                next_route: None,
            },
            devices,
        };
        network.refresh();
        network
    }

    fn bump(&mut self) {
        self.view.generation = self.view.generation.saturating_add(1);
    }

    fn interface_mut(
        &mut self,
        name: &str,
    ) -> Result<&mut syn_shell::network::NetworkInterfaceView, Status> {
        self.view
            .interfaces
            .iter_mut()
            .flatten()
            .find(|interface| interface.name.as_str().eq_ignore_ascii_case(name))
            .ok_or(Status::NOT_FOUND)
    }

    fn network_text(value: &str) -> Result<syn_shell::network::NetworkText, Status> {
        syn_shell::network::NetworkText::new(value).map_err(|_| Status::INVALID_ARGUMENT)
    }

    fn mac_text(mac: [u8; 6]) -> syn_shell::network::NetworkText {
        let mut text = syn_shell::network::NetworkText::empty();
        for (index, byte) in mac.iter().copied().enumerate() {
            if index != 0 {
                let _ = text.push_char(':');
            }
            let digits = b"0123456789abcdef";
            let _ = text.push_char(digits[(byte >> 4) as usize] as char);
            let _ = text.push_char(digits[(byte & 0x0f) as usize] as char);
        }
        text
    }

    fn queue_view(
        queue: synos_legacy_pc_drivers::EthernetQueueSnapshot,
    ) -> syn_shell::network::NetworkQueueView {
        syn_shell::network::NetworkQueueView {
            ready: queue.ready,
            head: queue.head,
            tail: queue.tail,
            capacity: queue.capacity,
        }
    }

    fn interface_name(index: usize) -> syn_shell::network::NetworkText {
        let mut name = syn_shell::network::NetworkText::empty();
        let _ = name.push_str("eth");
        let _ = name.push_char(b'0'.saturating_add(index as u8) as char);
        name
    }

    fn record_link_event(&mut self, index: usize, link_up: bool) {
        use syn_shell::network::{NetworkLinkEvent, MAX_NETWORK_LINK_EVENTS};

        let event = NetworkLinkEvent {
            generation: self.view.generation.saturating_add(1),
            interface: Self::interface_name(index),
            link_up,
        };
        if let Some(slot) = self.view.link_events.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(event);
            return
        }
        let mut index = 1;
        while index < MAX_NETWORK_LINK_EVENTS {
            self.view.link_events[index - 1] = self.view.link_events[index];
            index += 1;
        }
        self.view.link_events[MAX_NETWORK_LINK_EVENTS - 1] = Some(event);
    }

    fn refresh(&mut self) {
        let mut changed = false;
        for device_index in 0..self.devices.len() {
            let interface_index = device_index + 1;
            let snapshot = self.devices[device_index]
                .as_mut()
                .map(|device| device.snapshot());
            let Some(snapshot) = snapshot else {
                if self.view.interfaces[interface_index].take().is_some() {
                    changed = true;
                }
                continue
            };
            let name = Self::interface_name(device_index);
            let address = Self::network_text("0.0.0.0");
            let Some(address) = address.ok() else {
                continue
            };
            let current = self.view.interfaces[interface_index];
            let next = syn_shell::network::NetworkInterfaceView {
                name,
                address,
                prefix_len: current.and_then(|interface| interface.prefix_len),
                mac: Some(Self::mac_text(snapshot.mac)),
                gateway: current.and_then(|interface| interface.gateway),
                mtu: current.map_or(1500, |interface| interface.mtu),
                enabled: snapshot.admin_up,
                link_up: snapshot.link_up,
                rx_queue: Some(Self::queue_view(snapshot.rx_queue)),
                tx_queue: Some(Self::queue_view(snapshot.tx_queue)),
                mode: current.map_or(
                    syn_shell::network::InterfaceAddressMode::Static,
                    |interface| interface.mode,
                ),
                dhcp: current.and_then(|interface| interface.dhcp),
            };
            if current.is_some_and(|interface| interface.link_up != next.link_up) {
                self.record_link_event(device_index, next.link_up);
            }
            if current != Some(next) {
                changed = true;
            }
            self.view.interfaces[interface_index] = Some(next);
        }
        let count = self.view.interfaces.iter().flatten().count() as u64;
        if self.view.interface_count != count {
            self.view.interface_count = count;
        }
        if changed {
            self.bump()
        }
    }
}

#[allow(unused_mut)]
fn discover_runtime_network(
    boot_info: &'static BootInfo,
) -> [Option<synos_legacy_pc_drivers::EthernetRuntime>;
    syn_shell::network::MAX_NETWORK_OUTPUT_ROWS - 1] {
    let mut devices = core::array::from_fn(|_| None);
    #[cfg(target_arch = "x86_64")]
    {
        let mut config = synos_legacy_pc_drivers::pci::PortConfig;
        synos_legacy_pc_drivers::pci::enumerate(&mut config, |device| {
            let Some(candidate) = synos_legacy_pc_drivers::EthernetAdapter::from_pci(&device)
            else {
                return
            };
            if !matches!(
                candidate.kind,
                synos_legacy_pc_drivers::EthernetKind::IntelE1000
                    | synos_legacy_pc_drivers::EthernetKind::VirtioNet
            ) {
                return
            }
            let Some(runtime) = synos_legacy_pc_drivers::EthernetRuntime::open(
                candidate,
                boot_info.physical_address_offset,
            )
            .ok() else {
                return
            };
            if let Some(slot) = devices.iter_mut().find(|slot| slot.is_none()) {
                *slot = Some(runtime)
            }
        });
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = boot_info;
    }
    devices
}

impl syn_shell::network::NetworkSource for KernelNetwork {
    fn authorize_mutation(&mut self) -> Result<(), Status> {
        Ok(())
    }

    fn show_network(&mut self) -> Result<syn_shell::network::NetworkView, Status> {
        self.refresh();
        Ok(self.view)
    }

    fn show_interfaces(&mut self) -> Result<syn_shell::network::NetworkView, Status> {
        self.refresh();
        Ok(self.view)
    }

    fn show_routes(&mut self) -> Result<syn_shell::network::NetworkView, Status> {
        self.refresh();
        Ok(self.view)
    }

    fn set_hostname(
        &mut self,
        hostname: &str,
    ) -> Result<syn_shell::network::NetworkView, Status> {
        self.refresh();
        self.view.hostname = Some(Self::network_text(hostname)?);
        self.bump();
        Ok(self.view)
    }

    fn set_interface(
        &mut self,
        update: syn_shell::network::InterfaceUpdate<'_>,
    ) -> Result<syn_shell::network::NetworkView, Status> {
        use syn_shell::network::{DhcpLeaseView, InterfaceAddressMode};

        self.refresh();
        if let Some(enabled) = update.enabled {
            let interface_index = self
                .view
                .interfaces
                .iter()
                .flatten()
                .position(|interface| interface.name.as_str().eq_ignore_ascii_case(update.name))
                .ok_or(Status::NOT_FOUND)?;
            if interface_index != 0 {
                let device = self
                    .devices
                    .get_mut(interface_index - 1)
                    .and_then(Option::as_mut)
                    .ok_or(Status::NOT_FOUND)?;
                device.set_admin_up(enabled);
            }
        }
        let interface = self.interface_mut(update.name)?;
        if let Some(mode) = update.mode {
            interface.mode = mode;
            if mode == InterfaceAddressMode::Dhcp {
                if interface.address.as_str().is_empty() {
                    interface.address = Self::network_text("0.0.0.0")?;
                }
                interface.dhcp = Some(DhcpLeaseView {
                    state: Self::network_text("init")?,
                    server: None,
                    expires_at_ms: None,
                    dns0: None,
                    dns1: None,
                });
            } else {
                interface.dhcp = None;
            }
        }
        if let Some(address) = update.address {
            interface.address = Self::network_text(address)?;
            if update.mode.is_none() {
                interface.mode = InterfaceAddressMode::Static;
                interface.dhcp = None;
            }
        }
        if let Some(gateway) = update.gateway {
            interface.gateway = Some(Self::network_text(gateway)?);
        }
        if let Some(mtu) = update.mtu {
            interface.mtu = mtu;
        }
        if let Some(enabled) = update.enabled {
            interface.enabled = enabled;
        }
        self.bump();
        self.refresh();
        Ok(self.view)
    }

    fn set_route(
        &mut self,
        update: syn_shell::network::RouteUpdate<'_>,
    ) -> Result<syn_shell::network::NetworkView, Status> {
        use syn_shell::network::NetworkRouteView;

        self.refresh();
        self.interface_mut(update.interface)?;
        if let Some(existing) = self
            .view
            .routes
            .iter_mut()
            .flatten()
            .find(|route| route.destination.as_str() == update.destination)
        {
            existing.gateway = Self::network_text(update.gateway)?;
            existing.interface = Self::network_text(update.interface)?;
            if let Some(metric) = update.metric {
                existing.metric = metric;
            }
        } else {
            let slot = self
                .view
                .routes
                .iter_mut()
                .find(|slot| slot.is_none())
                .ok_or(Status::NO_SPACE)?;
            *slot = Some(NetworkRouteView {
                destination: Self::network_text(update.destination)?,
                gateway: Self::network_text(update.gateway)?,
                interface: Self::network_text(update.interface)?,
                metric: update.metric.unwrap_or(100),
            });
            self.view.route_count = self.view.route_count.saturating_add(1);
        }
        self.bump();
        Ok(self.view)
    }
}

struct KernelExecutor {
    boot_method: BootMethod,
    memory_regions: &'static [synos_boot_protocol::MemoryRegion],
    memory_region_count: usize,
    memory_total_bytes: u64,
    memory_available_bytes: u64,
    memory_used_bytes: u64,
    scheduler_clock: u64,
    acpi_ready: bool,
    generation: u64,
    completion: Option<(u64, Result<StructuredOutput, Status>)>,
    reboot_requested: bool,
    shutdown_requested: bool,
    monitor: MonitorState,
    scheduler: &'static mut Scheduler,
    capabilities: CapabilitySpace,
    control_authority: crate::CapabilityHandle,
    dlm: &'static DistributedLockManager<DEFAULT_LOCK_CAPACITY>,
    filesystem: FilesystemExecutor<KernelFilesystem>,
    network: KernelNetwork,
    firewall_policy_version: u64,
    firewall_rule_count: u64,
}

impl KernelExecutor {
    fn new(
        boot_info: &'static BootInfo,
        scheduler: &'static mut Scheduler,
        dlm: &'static DistributedLockManager<DEFAULT_LOCK_CAPACITY>,
        _node_fences: &'static NodeFenceTable<DEFAULT_NODE_FENCE_CAPACITY>,
        scheduler_clock: u64,
        acpi_ready: bool,
    ) -> Self {
        let memory_total_bytes = boot_info
            .regions()
            .iter()
            .fold(0u64, |total, region| total.saturating_add(region.length));
        let memory_available_bytes = boot_info
            .regions()
            .iter()
            .filter(|region| region.kind == MemoryKind::Usable)
            .fold(0u64, |total, region| total.saturating_add(region.length));

        let mut capabilities = CapabilitySpace::new();
        let control_authority = capabilities
            .mint_root(
                AddressSpaceId::KERNEL,
                CapabilityObject::SystemControl,
                Rights::CONTROL,
            )
            .expect("kernel control capability");
        let filesystem = KernelFilesystem::new();
        crate::println!("root filesystem mounted");

        Self {
            boot_method: boot_info.method,
            memory_regions: boot_info.regions(),
            memory_region_count: boot_info.memory_region_count,
            memory_total_bytes,
            memory_available_bytes,
            memory_used_bytes: memory_total_bytes.saturating_sub(memory_available_bytes),
            scheduler_clock,
            acpi_ready,
            generation: 0,
            completion: None,
            reboot_requested: false,
            shutdown_requested: false,
            monitor: MonitorState::new(),
            scheduler,
            capabilities,
            control_authority,
            dlm,
            filesystem: FilesystemExecutor::new(filesystem),
            network: KernelNetwork::new(boot_info),
            firewall_policy_version: 1,
            firewall_rule_count: 0,
        }
    }

    fn execute(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        match command.route.raw() {
            HELP_ROUTE => self.help(),
            SHOW_SYSTEM_ROUTE => self.show_system(),
            REBOOT_ROUTE => self.request_reboot(),
            SHUTDOWN_ROUTE => self.request_shutdown(),
            MONITOR_ROUTE => self.monitor_view(command.json()),
            SHOW_PROCESSES_ROUTE => self.show_processes(command.json()),
            TOP_CPU_ROUTE => self.top_cpu(command.json()),
            SHOW_MEMORY_ROUTE => self.show_memory(command.json()),
            SHOW_DSM_ROUTE => self.show_dsm(command.json()),
            UPTIME_ROUTE => self.uptime(),
            STOP_JOB_ROUTE => self.stop_job(command),
            SET_PROCESS_ROUTE => self.set_process(command),
            SYNOS_ISOLATE_ROUTE => self.isolate_cores(command),
            syn_shell::firewall::SHOW_FIREWALL_ROUTE => self.show_firewall(),
            syn_shell::firewall::SET_FIREWALL_ROUTE => self.set_firewall(command),
            route if (syn_shell::network::SHOW_NETWORK_ROUTE
                ..=syn_shell::network::SET_ROUTE_ROUTE)
                .contains(&route) =>
            {
                syn_shell::network::dispatch_network_command(&mut self.network, command)
            }
            route if (syn_shell::cluster::SHOW_CLUSTER_ROUTE
                ..=syn_shell::cluster::REMOVE_FEDERATION_ROUTE)
                .contains(&route) => syn_shell::cluster::execute_cluster_surface_command(
                command,
                None,
            ),
            route if (syn_shell::filesystem::DIRECTORY_ROUTE
                ..=syn_shell::filesystem::RMDIR_ROUTE)
                .contains(&route) =>
            {
                let result = self.filesystem.execute_command(command);
                if result.is_ok() {
                    self.filesystem.source_mut().persist();
                }
                result
            }
            _ => Err(Status::NOT_FOUND),
        }
    }

    fn show_firewall(&self) -> Result<StructuredOutput, Status> {
        syn_shell::firewall::firewall_output(syn_shell::firewall::FirewallView {
            policy_version: self.firewall_policy_version,
            rule_count: self.firewall_rule_count,
            active_connections: 0,
            dropped_packets: 0,
            allowed_packets: 0,
        })
    }

    fn set_firewall(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        let Some(Value::Text(rule)) = command.get("RULE") else {
            return Err(Status::INVALID_ARGUMENT)
        };
        if rule.is_empty() || self.firewall_rule_count >= 32 {
            return Err(Status::INVALID_ARGUMENT)
        }
        self.firewall_rule_count += 1;
        self.firewall_policy_version = self.firewall_policy_version.saturating_add(1);
        self.show_firewall()
    }

    fn print_directory(
        &mut self,
        command: CommandCall,
        keyboard: &mut crate::keyboard::Keyboard,
        usb_keyboard: &mut Option<crate::usb_keyboard::UsbKeyboard>,
    ) -> Result<(), Status> {
        let mut continuation = None;
        let mut printed_header = false;
        let mut printed_entries = false;

        loop {
            let mut page = DirectoryPage::new();
            let path = self
                .filesystem
                .list_directory_page(command, continuation, &mut page)?;
            if !printed_header {
                crate::println!("Directory: {}", path.as_str());
                crate::println!();
                crate::println!(
                    "\x1b[1;97;44m{:<20}  {:<10}  {:>8}  {:>7}\x1b[0m",
                    "NAME",
                    "TYPE",
                    "SIZE",
                    "VERSION",
                );
                printed_header = true;
            }
            for entry in page.entries() {
                if directory_cancelled(keyboard, usb_keyboard) {
                    crate::println!("^C");
                    return Ok(())
                }
                crate::println!(
                    "{:<20}  {:<10}  {:>8}  {:>7}",
                    entry.name.as_str(),
                    if entry.is_link { "LINK" } else { entry_type_name(entry.file_type) },
                    entry.size,
                    entry.version,
                );
                printed_entries = true;
            }
            let Some(next) = page.next else { break };
            continuation = Some(next);
        }

        if !printed_entries {
            crate::println!("(empty)");
        }
        Ok(())
    }

    fn print_type(
        &mut self,
        command: CommandCall,
        keyboard: &mut crate::keyboard::Keyboard,
        usb_keyboard: &mut Option<crate::usb_keyboard::UsbKeyboard>,
    ) -> Result<(), Status> {
        let binary = matches!(command.get("BINARY"), Some(Value::Boolean(true)));
        let mut output = TypeConsoleOutput::new(keyboard, usb_keyboard, binary);
        match self.filesystem.type_file_stream(command, &mut output) {
            Ok(_) => match output.finish() {
                Ok(()) => {
                    crate::println!();
                    Ok(())
                }
                Err(Status::BUSY) if output.cancelled => {
                    crate::println!("^C");
                    Ok(())
                }
                Err(status) => Err(status),
            },
            Err(Status::BUSY) if output.cancelled => {
                crate::println!("^C");
                Ok(())
            }
            Err(status) => Err(status),
        }
    }

    fn edit_file(
        &mut self,
        command: CommandCall,
        keyboard: &mut crate::keyboard::Keyboard,
        usb_keyboard: &mut Option<crate::usb_keyboard::UsbKeyboard>,
        acpi: Option<&AcpiPlatform>,
    ) -> Result<(), Status> {
        let mut contents = EditBufferOutput::<KERNEL_FILE_BYTES>::new();
        let metadata = self.filesystem.edit_file_load(command, &mut contents)?;
        self.filesystem.source_mut().persist();
        let mut editor = FileEditor::<KERNEL_FILE_BYTES>::new(
            metadata.path.as_str(),
            metadata.version,
            &contents.bytes[..contents.len],
        )
        .map_err(|_| Status::INVALID_ARGUMENT)?;

        crate::print!("\x1b[?1049h\x1b[2J\x1b[H");
        request_terminal_size();
        let result = self.run_edit_session(&mut editor, command, keyboard, usb_keyboard, acpi);
        restore_editor_terminal();
        match result {
            Ok(EditExit::Saved) => {
                print_edit_result("SAVED", &editor);
                Ok(())
            }
            Ok(EditExit::Discarded) => {
                print_edit_result("DISCARDED", &editor);
                Ok(())
            }
            Ok(EditExit::Cancelled) => {
                print_edit_result("CANCELLED", &editor);
                Ok(())
            }
            Err(status) => {
                crate::println!("EDIT operation=FAILED status={:#x}", status.raw());
                Err(status)
            }
        }
    }

    fn run_edit_session(
        &mut self,
        editor: &mut FileEditor<KERNEL_FILE_BYTES>,
        command: CommandCall,
        keyboard: &mut crate::keyboard::Keyboard,
        usb_keyboard: &mut Option<crate::usb_keyboard::UsbKeyboard>,
        acpi: Option<&AcpiPlatform>,
    ) -> Result<EditExit, Status> {
        render_file_editor(editor)?;
        let mut input = VtInput::new();
        let mut confirming_discard = None;
        let mut confirming_conflict = None;
        loop {
            let byte = wait_for_byte(keyboard, usb_keyboard, acpi);
            let mut key = input.advance(byte);
            if key.is_none() && input.escape_pending() {
                key = read_available_byte(keyboard, usb_keyboard)
                    .and_then(|next| input.advance(next));
                if key.is_none() {
                    key = input.flush_escape();
                }
            }
            let Some(key) = key else {
                continue
            };
            if let Some((columns, rows)) = input.take_resize() {
                crate::console::set_remote_terminal_size(columns, rows)
            }

            if let Some(exit) = confirming_discard {
                match key {
                    Key::Character('y') | Key::Character('Y') => return Ok(exit),
                    Key::Character('n') | Key::Character('N') | Key::Escape => {
                        confirming_discard = None;
                        render_file_editor(editor)?;
                    }
                    _ => {}
                }
                continue
            }

            if let Some(save_exit) = confirming_conflict {
                match key {
                    Key::Character('y') | Key::Character('Y') => {
                        match self.filesystem.edit_file_force_save(command, editor.bytes()) {
                            Ok(metadata) => {
                                editor.mark_saved(metadata.version);
                                self.filesystem.source_mut().persist();
                                confirming_conflict = None;
                                if save_exit {
                                    return Ok(EditExit::Saved)
                                }
                                render_file_editor(editor)?;
                            }
                            Err(status) => {
                                confirming_conflict = None;
                                render_file_editor(editor)?;
                                report_editor_error(status);
                            }
                        }
                    }
                    Key::Character('n') | Key::Character('N') | Key::Escape => {
                        confirming_conflict = None;
                        render_file_editor(editor)?;
                    }
                    _ => {}
                }
                continue
            }

            let action = match editor.handle(key) {
                Ok(action) => action,
                Err(error) => {
                    render_file_editor(editor)?;
                    report_editor_error(error.status());
                    continue
                }
            };
            match action {
                FileEditorAction::None => {}
                FileEditorAction::Redraw if is_cursor_only_edit_key(key) => {
                    render_file_editor_cursor(editor)?
                }
                FileEditorAction::Redraw if is_line_only_edit_key(editor, key) => {
                    render_file_editor_line(editor)?
                }
                FileEditorAction::Redraw => render_file_editor(editor)?,
                FileEditorAction::Save | FileEditorAction::SaveExit => {
                    let save_exit = action == FileEditorAction::SaveExit;
                    if !editor.is_dirty() {
                        if save_exit {
                            return Ok(EditExit::Saved)
                        }
                        render_file_editor(editor)?;
                        continue
                    }
                    match self
                        .filesystem
                        .edit_file_save_if_version(command, editor.version(), editor.bytes())
                    {
                        Ok(metadata) => {
                            editor.mark_saved(metadata.version);
                            self.filesystem.source_mut().persist();
                            if save_exit {
                                return Ok(EditExit::Saved)
                            }
                            render_file_editor(editor)?;
                        }
                        Err(Status::CONFLICT) => {
                            confirming_conflict = Some(save_exit);
                            render_file_editor(editor)?;
                            prompt_editor_conflict();
                        }
                        Err(status) => {
                            render_file_editor(editor)?;
                            report_editor_error(status);
                        }
                    }
                }
                FileEditorAction::PromptDiscard => {
                    confirming_discard = Some(if key == Key::Cancel {
                        EditExit::Cancelled
                    } else {
                        EditExit::Discarded
                    });
                    prompt_editor_discard();
                }
                FileEditorAction::DiscardExit => {
                    return Ok(if key == Key::Cancel {
                        EditExit::Cancelled
                    } else {
                        EditExit::Discarded
                    })
                }
            }
        }
    }

    fn help(&self) -> Result<StructuredOutput, Status> {
        let mut output = StructuredOutput::new(Status::NORMAL);
        insert_text(
            &mut output,
            "commands",
            "SHELL, SYSTEM, PROCESS, MEMORY, MONITOR, CONTROL, CLUSTER, FILESYSTEM, FIREWALL, NETWORK; use HELP <CATEGORY> for related commands; unique command prefixes accepted",
        )?;
        Ok(output)
    }

    fn print_help(&self, _registry: &CommandRegistry<COMMAND_CAPACITY>) {
        crate::println!("\x1b[1;97;44m  HELP  \x1b[0m");
        crate::println!("\x1b[1;97;44mCATEGORY     COMMAND\x1b[0m");
        for category in HELP_CATEGORIES {
            crate::print!("  ");
            crate::print!("{}", category);
            let mut pad = category.len();
            while pad < 12 {
                crate::print!(" ");
                pad += 1;
            }
            crate::print!(" HELP ");
            print_display_command(category);
            crate::println!();
        }
        crate::println!();
        crate::println!("Use HELP <CATEGORY> for related commands.");
        crate::println!("Use HELP <COMMAND> for syntax and parameters.");
        crate::println!("Add /JSON to any command for structured JSON output.");
        crate::println!("EDIT keys: Ctrl-S save, Ctrl-Z save and exit, Ctrl-X discard and exit.");
        crate::println!("  Insert text normally; Enter adds a line; Shift-arrows select text.");
        crate::println!("  Shift-Home/End extend selection; Backspace/Delete remove selected text.");
        crate::println!("  Escape enters command mode: I insert, S save, E save/exit, Q discard.");
        crate::println!("  Command mode: Y copy, X cut, P paste. Resize redraws the live terminal.");
        crate::println!();
        crate::println!("Unique command prefixes are accepted.");
    }

    #[inline(never)]
    fn print_command_help(
        &self,
        registry: &CommandRegistry<COMMAND_CAPACITY>,
        command: &str,
    ) {
        if let Some(category) = help_category(command) {
            self.print_category_help(registry, category);
            return
        }
        let Some(registration) = registry.registration(command) else {
            crate::println!("No help available for {command}.");
            return
        };

        crate::print!("\x1b[1;97;44m  HELP: ");
        print_display_command(registration.spec.name.as_str());
        crate::println!("  \x1b[0m");
        crate::print!("SYNTAX: ");
        print_display_command(registration.spec.name.as_str());
        for argument in registration.spec.arguments().filter(|argument| argument.positional) {
            if argument.required {
                crate::print!(" <{}>", argument.name.as_str());
            } else {
                crate::print!(" [<{}>]", argument.name.as_str());
            }
        }
        for argument in registration.spec.arguments().filter(|argument| !argument.positional) {
            if argument.kind == ArgumentKind::Boolean {
                if argument.required {
                    crate::print!(" /{}", argument.name.as_str());
                } else {
                    crate::print!(" [/{}]", argument.name.as_str());
                }
            } else {
                if argument.required {
                    crate::print!(
                        " /{}=<{}>",
                        argument.name.as_str(),
                        argument_kind(argument.kind)
                    );
                } else {
                    crate::print!(
                        " [/{}=<{}>]",
                        argument.name.as_str(),
                        argument_kind(argument.kind)
                    );
                }
            }
        }
        crate::println!();
        crate::println!("PARAMETERS:");
        let mut has_boolean = false;
        let mut has_arguments = false;
        for argument in registration.spec.arguments() {
            has_arguments = true;
            has_boolean |= argument.kind == ArgumentKind::Boolean;
            crate::print!("  ");
            crate::print!("{}", argument.name.as_str());
            crate::print!(" type={}", argument_kind(argument.kind));
            crate::print!(
                " {}",
                if argument.required { "required" } else { "optional" }
            );
            crate::println!(
                " {}",
                if argument.positional { "positional" } else { "qualifier" }
            );
        }
        if !has_arguments {
            crate::println!("  none");
        }
        if has_boolean {
            crate::println!("Boolean parameters accept TRUE, FALSE, YES, NO, 1, or 0.");
        }
    }

    #[inline(never)]
    fn print_category_help(
        &self,
        registry: &CommandRegistry<COMMAND_CAPACITY>,
        category: &str,
    ) {
        crate::print!("\x1b[1;97;44m  HELP: ");
        print_display_command(category);
        crate::println!("  \x1b[0m");
        for registration in registry.registrations() {
            if command_category(registration.route.raw()) == category {
                crate::print!("  ");
                print_display_command(registration.spec.name.as_str());
                print_command_syntax(&registration.spec);
                crate::println!();
            }
        }
        crate::println!();
        crate::println!("Use HELP <COMMAND> for parameters.");
    }

    fn show_system(&self) -> Result<StructuredOutput, Status> {
        let mut output = StructuredOutput::new(Status::NORMAL);
        insert_text(&mut output, "name", "SynOS")?;
        insert_text(&mut output, "architecture", architecture())?;
        insert_text(
            &mut output,
            "boot-method",
            match self.boot_method {
                BootMethod::Bios => "BIOS",
                BootMethod::Uefi => "UEFI",
            },
        )?;
        insert(
            &mut output,
            "memory-regions",
            OutputValue::Unsigned(self.memory_region_count as u64),
        )?;
        insert(
            &mut output,
            "scheduler-clock",
            OutputValue::Unsigned(self.scheduler_clock),
        )?;
        insert_text(
            &mut output,
            "acpi",
            if self.acpi_ready { "ready" } else { "unavailable" },
        )?;
        insert_text(&mut output, "shell", "ready")?;
        insert_text(&mut output, "monitor", "active")?;
        Ok(output)
    }

    fn print_system(&self) {
        crate::println!("\x1b[1;97;44mPROPERTY             VALUE\x1b[0m");
        crate::println!("Name                 SynOS");
        crate::println!("Architecture         {}", architecture());
        crate::println!(
            "Boot method          {}",
            match self.boot_method {
                BootMethod::Bios => "BIOS",
                BootMethod::Uefi => "UEFI",
            }
        );
        crate::println!("Memory regions       {}", self.memory_region_count);
        crate::println!("Scheduler clock      {}", self.scheduler_clock);
        crate::println!(
            "ACPI                 {}",
            if self.acpi_ready { "ready" } else { "unavailable" }
        );
        crate::println!("Shell                ready");
        crate::println!("Monitor              active");
        crate::println!()
    }

    fn request_reboot(&mut self) -> Result<StructuredOutput, Status> {
        self.filesystem.source_mut().persist();
        self.reboot_requested = true;
        let mut output = StructuredOutput::new(Status::NORMAL);
        insert_text(&mut output, "action", "rebooting")?;
        Ok(output)
    }

    fn take_reboot_requested(&mut self) -> bool {
        core::mem::take(&mut self.reboot_requested)
    }

    fn request_shutdown(&mut self) -> Result<StructuredOutput, Status> {
        self.filesystem.source_mut().persist();
        self.shutdown_requested = true;
        let mut output = StructuredOutput::new(Status::NORMAL);
        insert_text(&mut output, "action", "shutting-down")?;
        Ok(output)
    }

    fn stop_job(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        if !matches!(
            command.get("JOB"),
            Some(Value::Text(value)) if value.as_str().eq_ignore_ascii_case("JOB")
        ) {
            return Err(Status::INVALID_ARGUMENT)
        }
        let id = thread_id(command.get("ID"))?;
        self.scheduler
            .terminate(
                &self.capabilities,
                AddressSpaceId::KERNEL,
                self.control_authority,
                id,
            )
            .map_err(|error| error.status())?;
        let mut output = StructuredOutput::new(Status::NORMAL);
        insert_text(&mut output, "action", "stopped")?;
        insert(&mut output, "job", OutputValue::Unsigned(id.raw() as u64))?;
        Ok(output)
    }

    fn set_process(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        let id = thread_id(command.get("ID"))?;
        let priority = priority(command.get("PRIORITY"))?;
        self.scheduler
            .set_priority(
                &self.capabilities,
                AddressSpaceId::KERNEL,
                self.control_authority,
                id,
                priority,
            )
            .map_err(|error| error.status())?;
        let mut output = StructuredOutput::new(Status::NORMAL);
        insert_text(&mut output, "action", "priority-updated")?;
        insert(&mut output, "process", OutputValue::Unsigned(id.raw() as u64))?;
        insert(
            &mut output,
            "priority",
            OutputValue::Unsigned(priority as u64),
        )?;
        Ok(output)
    }

    fn isolate_cores(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        let cpus = parse_cpu_mask(command.get_text("CORES").ok_or(Status::INVALID_ARGUMENT)?)?;
        self.scheduler
            .isolate_cores(
                &self.capabilities,
                AddressSpaceId::KERNEL,
                self.control_authority,
                cpus,
            )
            .map_err(|error| error.status())?;
        let mut output = StructuredOutput::new(Status::NORMAL);
        insert_text(&mut output, "action", "cores-isolated")?;
        insert(&mut output, "cores", OutputValue::Unsigned(cpus.raw()))?;
        insert(
            &mut output,
            "housekeeping",
            OutputValue::Unsigned(self.scheduler.partition().housekeeping().raw()),
        )?;
        Ok(output)
    }

    fn take_shutdown_requested(&mut self) -> bool {
        core::mem::take(&mut self.shutdown_requested)
    }

    fn monitor_view(&mut self, json: bool) -> Result<StructuredOutput, Status> {
        self.monitor.update();
        let view = self.monitor.current_view();
        let output = match view {
            MonitorView::Processes => self.show_processes(json),
            MonitorView::TopCpu => self.top_cpu(json),
            MonitorView::Dsm => self.show_dsm(json),
            MonitorView::Memory => self.show_memory(json),
        };
        self.monitor.switch_view();
        output
    }

    fn show_processes(&self, json: bool) -> Result<StructuredOutput, Status> {
        let mut output = StructuredOutput::new(Status::NORMAL);
        insert_text(&mut output, "view", "processes")?;
        let processes = MonitorState::get_processes(self.scheduler);
        if !json {
            crate::println!("\x1b[1;97;46mTHREAD       STATE      SWITCHES SPACE     POLICY\x1b[0m");
        }
        let mut idx: u64 = 0;
        for proc in processes.iter() {
            if let Some(p) = proc {
                let state_str = match p.state {
                    crate::task::ThreadState::Vacant => "VACANT",
                    crate::task::ThreadState::Ready => "READY",
                    crate::task::ThreadState::Running => "RUNNING",
                    crate::task::ThreadState::Blocked => "BLOCKED",
                    crate::task::ThreadState::Sleeping => "SLEEPING",
                };
                let policy_str = match p.policy {
                    crate::task::SchedulingPolicy::Cooperative => "COOP",
                    crate::task::SchedulingPolicy::Realtime { priority: _, .. } => "RT",
                };
                if !json {
                    crate::println!("{} {:<10} {:<8} {:<8} {:<10}",
                        p.thread_id.raw(),
                        state_str,
                        p.switches,
                        p.address_space.raw(),
                        policy_str
                    );
                }
                idx += 1;
            }
        }
        if !json {
            crate::println!("Total: {} processes", idx);
        }
        insert(&mut output, "process-count", OutputValue::Unsigned(idx))?;
        Ok(output)
    }

    fn top_cpu(&mut self, json: bool) -> Result<StructuredOutput, Status> {
        let mut output = StructuredOutput::new(Status::NORMAL);
        insert_text(&mut output, "view", "cpu")?;
        if !json {
            self.print_top_cpu();
        }
        Ok(output)
    }

    fn print_top_cpu(&mut self) {
        let top = MonitorState::get_top_cpu(self.scheduler, &mut self.monitor.cpu_history);
        crate::println!("\x1b[1;97;42mTHREAD     OWNER        SPACE    STATE    POLICY  CPU%  SWITCHES\x1b[0m");
        let mut idx: u64 = 0;
        for cpu in top.iter().flatten() {
            let state = match cpu.state {
                crate::task::ThreadState::Ready => "READY",
                crate::task::ThreadState::Running => "RUNNING",
                crate::task::ThreadState::Blocked => "BLOCKED",
                crate::task::ThreadState::Sleeping => "SLEEPING",
                crate::task::ThreadState::Vacant => "VACANT",
            };
            let policy = match cpu.policy {
                crate::task::SchedulingPolicy::Cooperative => "COOP",
                crate::task::SchedulingPolicy::Realtime { .. } => "RT",
            };
            if cpu.owner == 0 {
                crate::println!("{:08x}  {:<12}  0x{:04x}  {:<8} {:<7} {:>3}%  {:>8}",
                    cpu.thread_id.raw(),
                    "anonymous",
                    cpu.address_space.raw(),
                    state,
                    policy,
                    cpu.util_percent,
                    cpu.switches,
                );
            } else {
                crate::println!("{:08x}  id:{:<8}  0x{:04x}  {:<8} {:<7} {:>3}%  {:>8}",
                    cpu.thread_id.raw(),
                    cpu.owner,
                    cpu.address_space.raw(),
                    state,
                    policy,
                    cpu.util_percent,
                    cpu.switches,
                );
            }
            idx += 1;
        }
        if idx == 0 {
            crate::println!("No active threads.");
        }
        crate::println!("Total: {} threads", idx);
        crate::println!("CPU% is scheduler activity share since the last sample.");
    }

    fn show_memory(&self, json: bool) -> Result<StructuredOutput, Status> {
        let mut output = StructuredOutput::new(Status::NORMAL);
        insert_text(&mut output, "view", "memory")?;
        insert(
            &mut output,
            "total-bytes",
            OutputValue::Unsigned(self.memory_total_bytes),
        )?;
        insert(
            &mut output,
            "available-bytes",
            OutputValue::Unsigned(self.memory_available_bytes),
        )?;
        insert(
            &mut output,
            "used-bytes",
            OutputValue::Unsigned(self.memory_used_bytes),
        )?;
        insert(
            &mut output,
            "memory-regions",
            OutputValue::Unsigned(self.memory_region_count as u64),
        )?;
        if !json {
            self.print_memory();
        }
        Ok(output)
    }

    fn print_memory(&self) {
        crate::println!("\x1b[1;97;44mPROPERTY             VALUE\x1b[0m");
        crate::println!("\x1b[1mTotal\x1b[0m                {}", memory_size(self.memory_total_bytes));
        crate::println!(
            "\x1b[1mUsed\x1b[0m                 {}  ({:>4}.{}%)",
            memory_size(self.memory_used_bytes),
            memory_percent(self.memory_used_bytes, self.memory_total_bytes) / 10,
            memory_percent(self.memory_used_bytes, self.memory_total_bytes) % 10,
        );
        crate::println!(
            "\x1b[1mAvailable\x1b[0m            {}  ({:>4}.{}%)",
            memory_size(self.memory_available_bytes),
            memory_percent(self.memory_available_bytes, self.memory_total_bytes) / 10,
            memory_percent(self.memory_available_bytes, self.memory_total_bytes) % 10,
        );
        crate::println!("\x1b[1mRegions\x1b[0m              {}", self.memory_region_count);
        crate::println!();
        for (index, region) in self.memory_regions.iter().enumerate() {
            let available_bytes = region_available_bytes(region.kind, region.length);
            let used_bytes = region_used_bytes(region.kind, region.length);
            crate::println!(
                "\x1b[1;96mRegion {}             {}\x1b[0m",
                index,
                memory_kind_name(region.kind),
            );
            crate::println!("  \x1b[1mUsed\x1b[0m                {}", memory_size(used_bytes));
            crate::println!("  \x1b[1mAvailable\x1b[0m           {}", memory_size(available_bytes));
        }
    }

    fn show_dsm(&self, json: bool) -> Result<StructuredOutput, Status> {
        let locks = MonitorState::get_lock_contentions(self.dlm);

        let mut active_locks = 0u64;
        let mut granted_locks = 0u64;
        let mut queued_locks = 0u64;
        for lock in locks.iter() {
            if lock.resource_id.is_some() {
                active_locks += 1;
                granted_locks += lock.granted as u64;
                queued_locks += lock.queued as u64;
            }
        }

        let mut output = StructuredOutput::new(Status::NORMAL);
        insert_text(&mut output, "view", "dsm")?;
        insert(&mut output, "active-locks", OutputValue::Unsigned(active_locks))?;
        insert(&mut output, "granted-locks", OutputValue::Unsigned(granted_locks))?;
        insert(&mut output, "queued-locks", OutputValue::Unsigned(queued_locks))?;

        if !json {
            crate::println!("\x1b[1;97;43mSTAT                 VALUE\x1b[0m");
            crate::println!("Active locks         {} / {}", active_locks, MAX_LOCKS);
            crate::println!("Granted              {}", granted_locks);
            crate::println!("Queued               {}", queued_locks);
        }

        if active_locks == 0 {
            if !json {
                crate::println!("No active DSM locks.");
            }
            return Ok(output)
        }

        if !json {
            crate::println!();
            crate::println!("\x1b[1;97;43mRESOURCE   STATE    QUEUED  OWNER NODE\x1b[0m");
        }
        for lock in locks.iter() {
            let Some(resource_id) = lock.resource_id else { continue };
            let state = if lock.granted != 0 { "GRANTED" } else { "WAITING" };
            if !json {
                crate::println!("{:<10} {:<8} {:<7} {}",
                    resource_id.raw(),
                    state,
                    lock.queued,
                    lock.owner_node
                );
            }
        }
        Ok(output)
    }

    fn uptime(&self) -> Result<StructuredOutput, Status> {
        const SECONDS_PER_MINUTE: u64 = 60;
        const SECONDS_PER_HOUR: u64 = 60 * SECONDS_PER_MINUTE;
        const SECONDS_PER_DAY: u64 = 24 * SECONDS_PER_HOUR;

        let uptime_us = self.scheduler.clock();
        let total_seconds = uptime_us / 1_000_000;
        let days = total_seconds / SECONDS_PER_DAY;
        let hours = (total_seconds % SECONDS_PER_DAY) / SECONDS_PER_HOUR;
        let minutes = (total_seconds % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE;
        let seconds = total_seconds % SECONDS_PER_MINUTE;
        let mut output = StructuredOutput::new(Status::NORMAL);
        insert(&mut output, "uptime-us", OutputValue::Unsigned(uptime_us))?;
        insert(&mut output, "days", OutputValue::Unsigned(days))?;
        insert(&mut output, "hours", OutputValue::Unsigned(hours))?;
        insert(&mut output, "minutes", OutputValue::Unsigned(minutes))?;
        insert(&mut output, "seconds", OutputValue::Unsigned(seconds))?;
        Ok(output)
    }
}

impl CommandExecutor for KernelExecutor {
    fn submit(
        &mut self,
        command: CommandCall,
        pipeline_input: Option<&StructuredOutput>,
    ) -> Result<ExecutionToken, Error> {
        if self.completion.is_some() {
            return Err(Error::AlreadyRunning);
        }
        self.generation = self.generation.wrapping_add(1).max(1);
        let token = ExecutionToken::new(self.generation).ok_or(Error::InvalidHandle)?;
        let completion = if (syn_shell::cluster::SHOW_CLUSTER_ROUTE
            ..=syn_shell::cluster::REMOVE_FEDERATION_ROUTE)
            .contains(&command.route.raw())
        {
            syn_shell::cluster::execute_cluster_surface_command(command, pipeline_input)
        } else {
            self.execute(command)
        };
        self.completion = Some((token.raw(), completion));
        Ok(token)
    }

    fn poll(&mut self, token: ExecutionToken) -> Option<Result<StructuredOutput, Status>> {
        let (raw, completion) = self.completion.take()?;
        if raw == token.raw() {
            Some(completion)
        } else {
            self.completion = Some((raw, completion));
            Some(Err(Status::NOT_FOUND))
        }
    }

    fn cancel(&mut self, token: ExecutionToken) -> Result<(), Error> {
        let Some((raw, _)) = self.completion.as_ref() else {
            return Err(Error::InvalidHandle);
        };
        if *raw != token.raw() {
            return Err(Error::InvalidHandle);
        }
        self.completion = None;
        Ok(())
    }
}

fn insert_text(output: &mut StructuredOutput, name: &str, value: &str) -> Result<(), Status> {
    let text = OutputText::new(value).map_err(|_| Status::NO_SPACE)?;
    insert(output, name, OutputValue::Text(text))
}

fn thread_id(value: Option<Value>) -> Result<ThreadId, Status> {
    let Value::Integer(raw) = value.ok_or(Status::INVALID_ARGUMENT)? else {
        return Err(Status::INVALID_ARGUMENT)
    };
    if raw < 0 || raw > u32::MAX as i64 {
        return Err(Status::INVALID_ARGUMENT)
    }
    ThreadId::new(raw as u32).ok_or(Status::INVALID_ARGUMENT)
}

fn priority(value: Option<Value>) -> Result<u8, Status> {
    let Value::Integer(raw) = value.ok_or(Status::INVALID_ARGUMENT)? else {
        return Err(Status::INVALID_ARGUMENT)
    };
    if !(1..=u8::MAX as i64).contains(&raw) {
        return Err(Status::INVALID_ARGUMENT)
    }
    Ok(raw as u8)
}

fn status_reason(status: Status) -> &'static str {
    if status == Status::INVALID_ARGUMENT {
        "invalid argument"
    } else if status == Status::NOT_FOUND {
        "path not found"
    } else if status == Status::ALREADY_EXISTS {
        "path already exists"
    } else if status == Status::NO_SPACE {
        "no space left"
    } else if status == Status::ACCESS_DENIED {
        "access denied"
    } else if status == Status::CONFLICT {
        "file changed since edit began"
    } else if status == Status::DIRECTORY_NOT_EMPTY {
        "directory is not empty"
    } else if status == Status::INVALID_PATH {
        "invalid path"
    } else if status == Status::NOT_DIRECTORY {
        "not a directory"
    } else if status == Status::READ_ONLY {
        "read-only mount"
    } else {
        status.message()
    }
}

fn insert(output: &mut StructuredOutput, name: &str, value: OutputValue) -> Result<(), Status> {
    output.insert(name, value).map_err(|_| Status::NO_SPACE)
}

fn memory_size(bytes: u64) -> MemorySize {
    let (divisor, unit) = if bytes >= BYTES_PER_GIB {
        (BYTES_PER_GIB, "GiB")
    } else if bytes >= BYTES_PER_MIB {
        (BYTES_PER_MIB, "MiB")
    } else if bytes >= BYTES_PER_KIB {
        (BYTES_PER_KIB, "KiB")
    } else {
        (1, "bytes")
    };
    let whole = bytes / divisor;
    let tenth = bytes % divisor * 10 + divisor / 2;
    if tenth >= divisor {
        MemorySize(whole + 1, 0, bytes, unit)
    } else {
        MemorySize(whole, tenth / divisor, bytes, unit)
    }
}

fn memory_percent(value: u64, total: u64) -> u64 {
    if total == 0 {
        0
    } else {
        value.saturating_mul(1000) / total
    }
}

struct MemorySize(u64, u64, u64, &'static str);

impl core::fmt::Display for MemorySize {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.3 == "bytes" {
            write!(formatter, "{} bytes", self.2)
        } else {
            write!(formatter, "{}.{} {} ({} bytes)", self.0, self.1, self.3, self.2)
        }
    }
}

const BYTES_PER_KIB: u64 = 1024;
const BYTES_PER_MIB: u64 = 1024 * 1024;
const BYTES_PER_GIB: u64 = 1024 * 1024 * 1024;

fn region_available_bytes(kind: MemoryKind, length: u64) -> u64 {
    if kind == MemoryKind::Usable { length } else { 0 }
}

fn region_used_bytes(kind: MemoryKind, length: u64) -> u64 {
    if kind == MemoryKind::Usable { 0 } else { length }
}

const fn memory_kind_name(kind: MemoryKind) -> &'static str {
    match kind {
        MemoryKind::Usable => "USABLE",
        MemoryKind::Reserved => "RESERVED",
        MemoryKind::AcpiReclaimable => "ACPI RECLAIMABLE",
        MemoryKind::AcpiNonVolatile => "ACPI NONVOLATILE",
        MemoryKind::Bootloader => "BOOTLOADER",
        MemoryKind::Kernel => "KERNEL",
        MemoryKind::Framebuffer => "FRAMEBUFFER",
    }
}

const fn architecture() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    {
        "x86_64"
    }
    #[cfg(target_arch = "aarch64")]
    {
        "aarch64"
    }
    #[cfg(target_arch = "riscv64")]
    {
        "riscv64"
    }
    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64"
    )))]
    {
        "unknown"
    }
}
