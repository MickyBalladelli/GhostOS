use syn_shell::{
    Error,
    editor::{EditorAction, Key, LineEditor},
    interpreter::{CommandExecutor, ExecutionToken, Interpreter, InterpreterEvent},
    parser::{CommandCall, CommandRegistry, RouteId},
    render::{OutputFormat, render},
};
use synos_boot_protocol::{BootInfo, BootMethod};
use synos_status::Status;
use synos_system_model::command::{
    CommandSpec, OutputText, OutputValue, StructuredOutput,
};

const HELP_ROUTE: u16 = 1;
const SHOW_SYSTEM_ROUTE: u16 = 2;
const REBOOT_ROUTE: u16 = 3;
const COMMAND_CAPACITY: usize = 3;
const HISTORY_CAPACITY: usize = 8;

pub fn run(boot_info: &'static BootInfo, scheduler_clock: u64) -> ! {
    let mut registry = CommandRegistry::<COMMAND_CAPACITY>::new();
    register(&mut registry, "HELP", HELP_ROUTE);
    register(&mut registry, "SHOW-SYSTEM", SHOW_SYSTEM_ROUTE);
    register(&mut registry, "REBOOT", REBOOT_ROUTE);

    let mut editor = LineEditor::<HISTORY_CAPACITY>::new();
    let mut interpreter = Interpreter::new();
    let mut executor = KernelExecutor::new(boot_info, scheduler_clock);
    let mut keyboard = crate::keyboard::Keyboard::new();
    let mut usb_keyboard = crate::usb_keyboard::UsbKeyboard::new();
    let mut ignore_line_feed = false;

    crate::console::clear();
    crate::println!(r"   _____             ____   _____");
    crate::println!(r"  / ____|           / __ \ / ____|");
    crate::println!(r" | (___  _   _ _ __| |  | | (___");
    crate::println!(r"  \___ \| | | | '_ \ |  | |\___ \");
    crate::println!(r"  ____) | |_| | | | | |__| |____) |");
    crate::println!(r" |_____/ \__, |_| |_|\____/|_____/");
    crate::println!(r"          __/ |");
    crate::println!(r"         |___/");
    crate::println!();
    crate::println!();
    crate::println!();
    prompt();

    loop {
        let byte = wait_for_byte(&mut keyboard, &mut usb_keyboard);
        if ignore_line_feed && byte == b'\n' {
            ignore_line_feed = false;
            continue
        }
        ignore_line_feed = false;

        let key = match byte {
            b'\r' => {
                ignore_line_feed = true;
                Key::Enter
            }
            b'\n' => Key::Enter,
            8 | 127 => Key::Backspace,
            3 => Key::Cancel,
            0x20..=0x7e => Key::Character(byte as char),
            _ => continue,
        };
        let old_length = editor.line().len();

        match editor.handle(key) {
            Ok(EditorAction::Redraw) => match key {
                Key::Character(character) => crate::print!("{character}"),
                Key::Backspace if editor.line().len() < old_length => {
                    crate::print!("\x08 \x08")
                }
                _ => {}
            },
            Ok(EditorAction::Submit(line)) => {
                crate::println!();
                if !line.as_str().trim().is_empty() {
                    execute_line(
                        line.as_str(),
                        &registry,
                        &mut interpreter,
                        &mut executor,
                    )
                }
                prompt()
            }
            Ok(EditorAction::Cancel) => {
                crate::println!("^C");
                prompt()
            }
            Ok(EditorAction::None) => {}
            Err(error) => {
                crate::println!();
                crate::println!("shell input error: {error:?}");
                editor.clear();
                prompt()
            }
        }
    }
}

fn register(
    registry: &mut CommandRegistry<COMMAND_CAPACITY>,
    name: &str,
    route: u16,
) {
    let command = CommandSpec::new(name, &[]).expect("valid kernel command");
    let route = RouteId::new(route).expect("nonzero kernel route");
    registry
        .register(command, route)
        .expect("kernel command registry has capacity")
}

fn execute_line(
    line: &str,
    registry: &CommandRegistry<COMMAND_CAPACITY>,
    interpreter: &mut Interpreter,
    executor: &mut KernelExecutor,
) {
    let program = match registry.parse(line) {
        Ok(program) => program,
        Err(error) => {
            crate::println!("shell error: {error:?}");
            return
        }
    };
    if program.background {
        crate::println!("shell error: background jobs not ready");
        return
    }

    match interpreter.start_program(program, executor) {
        Ok(InterpreterEvent::Started) => {}
        Ok(_) => {
            crate::println!("shell error: command did not start");
            return
        }
        Err(error) => {
            crate::println!("shell error: {error:?}");
            return
        }
    }

    while interpreter.is_running() {
        match interpreter.poll(executor) {
            Ok(InterpreterEvent::Pending) => crate::arch::halt(),
            Ok(InterpreterEvent::Complete(output)) => {
                match render(&output, OutputFormat::List) {
                    Ok(text) => crate::print!("{}", text.as_str()),
                    Err(error) => crate::println!("shell output error: {error:?}"),
                }
            }
            Ok(InterpreterEvent::Failed(status)) => {
                crate::println!("command failed: status={:#x}", status.raw())
            }
            Ok(InterpreterEvent::Cancelled) => crate::println!("command cancelled"),
            Ok(InterpreterEvent::Submitted(job)) => {
                crate::println!("background job {} submitted", job.raw())
            }
            Ok(InterpreterEvent::Started) => {}
            Err(error) => {
                crate::println!("shell error: {error:?}");
                break
            }
        }
    }

    if executor.take_reboot_requested() {
        reboot()
    }
}

fn prompt() {
    crate::print!("$ ")
}

fn wait_for_byte(
    keyboard: &mut crate::keyboard::Keyboard,
    usb_keyboard: &mut Option<crate::usb_keyboard::UsbKeyboard>,
) -> u8 {
    loop {
        if let Some(byte) = keyboard.read_byte() {
            return byte
        }
        if let Some(byte) = usb_keyboard
            .as_mut()
            .and_then(crate::usb_keyboard::UsbKeyboard::read_byte)
        {
            return byte
        }
        if let Some(byte) = crate::console::read_byte() {
            return byte
        }
        crate::arch::halt()
    }
}

struct KernelExecutor {
    boot_method: BootMethod,
    memory_region_count: usize,
    scheduler_clock: u64,
    generation: u64,
    completion: Option<(u64, Result<StructuredOutput, Status>)>,
    reboot_requested: bool,
}

impl KernelExecutor {
    fn new(boot_info: &BootInfo, scheduler_clock: u64) -> Self {
        Self {
            boot_method: boot_info.method,
            memory_region_count: boot_info.memory_region_count,
            scheduler_clock,
            generation: 0,
            completion: None,
            reboot_requested: false,
        }
    }

    fn execute(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        match command.route.raw() {
            HELP_ROUTE => self.help(),
            SHOW_SYSTEM_ROUTE => self.show_system(),
            REBOOT_ROUTE => self.request_reboot(),
            _ => Err(Status::NOT_FOUND),
        }
    }

    fn help(&self) -> Result<StructuredOutput, Status> {
        let mut output = StructuredOutput::new(Status::NORMAL);
        insert_text(
            &mut output,
            "commands",
            "HELP, SHOW SYSTEM, REBOOT",
        )?;
        Ok(output)
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
        insert_text(&mut output, "shell", "ready")?;
        Ok(output)
    }

    fn request_reboot(&mut self) -> Result<StructuredOutput, Status> {
        self.reboot_requested = true;
        let mut output = StructuredOutput::new(Status::NORMAL);
        insert_text(&mut output, "action", "rebooting")?;
        Ok(output)
    }

    fn take_reboot_requested(&mut self) -> bool {
        core::mem::take(&mut self.reboot_requested)
    }
}

impl CommandExecutor for KernelExecutor {
    fn submit(
        &mut self,
        command: CommandCall,
        _pipeline_input: Option<&StructuredOutput>,
    ) -> Result<ExecutionToken, Error> {
        if self.completion.is_some() {
            return Err(Error::AlreadyRunning)
        }
        self.generation = self.generation.wrapping_add(1).max(1);
        let token = ExecutionToken::new(self.generation).ok_or(Error::InvalidHandle)?;
        let completion = self.execute(command);
        self.completion = Some((token.raw(), completion));
        Ok(token)
    }

    fn poll(
        &mut self,
        token: ExecutionToken,
    ) -> Option<Result<StructuredOutput, Status>> {
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
            return Err(Error::InvalidHandle)
        };
        if *raw != token.raw() {
            return Err(Error::InvalidHandle)
        }
        self.completion = None;
        Ok(())
    }
}

fn insert_text(
    output: &mut StructuredOutput,
    name: &str,
    value: &str,
) -> Result<(), Status> {
    let text = OutputText::new(value).map_err(|_| Status::NO_SPACE)?;
    insert(output, name, OutputValue::Text(text))
}

fn insert(
    output: &mut StructuredOutput,
    name: &str,
    value: OutputValue,
) -> Result<(), Status> {
    output.insert(name, value).map_err(|_| Status::NO_SPACE)
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

fn reboot() -> ! {
    crate::println!("Rebooting SynOS...");

    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let mut attempts = 100_000;
        while attempts != 0 && inb(0x64) & 0x02 != 0 {
            attempts -= 1;
            core::hint::spin_loop()
        }
        outb(0x64, 0xfe)
    }

    crate::halt()
}

#[cfg(target_arch = "x86_64")]
unsafe fn outb(port: u16, value: u8) {
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") port,
            in("al") value,
            options(nomem, nostack)
        )
    }
}

#[cfg(target_arch = "x86_64")]
unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    unsafe {
        core::arch::asm!(
            "in al, dx",
            out("al") value,
            in("dx") port,
            options(nomem, nostack)
        )
    }
    value
}
