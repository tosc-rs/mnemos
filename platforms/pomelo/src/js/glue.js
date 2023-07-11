export function to_term(s) {
    // console.log("TO TERM", s);
    window.term.write(s);
}

export function init_term(command_callback) {
    var baseTheme = {
        foreground: '#F8F8F8',
        background: '#2D2E2C',
        selection: '#5DA5D533',
        black: '#1E1E1D',
        brightBlack: '#262625',
        red: '#CE5C5C',
        brightRed: '#FF7272',
        green: '#5BCC5B',
        brightGreen: '#72FF72',
        yellow: '#CCCC5B',
        brightYellow: '#FFFF72',
        blue: '#5D5DD3',
        brightBlue: '#7279FF',
        magenta: '#BC5ED1',
        brightMagenta: '#E572FF',
        cyan: '#5DA5D5',
        brightCyan: '#72F0FF',
        white: '#F8F8F8',
        brightWhite: '#FFFFFF'
    };

    var term = new Terminal({
        fontFamily: '"Cascadia Code", Menlo, monospace',
        theme: baseTheme,
        cursorBlink: true,
        allowProposedApi: true
    });
    term.open(document.getElementById('terminal'));

    window.term = term;

    // var isWebglEnabled = false;
    // try {
    //     const webgl = new window.WebglAddon.WebglAddon();
    //     term.loadAddon(webgl);
    //     isWebglEnabled = true;
    // } catch (e) {
    //     console.warn('WebGL addon threw an exception during load', e);
    // }

    // Cancel wheel events from scrolling the page if the terminal has scrollback
    document.querySelector('.xterm').addEventListener('wheel', e => {
        if (term.buffer.active.baseY > 0) {
            e.preventDefault();
        }
    });


    function runFakeTerminal() {
        if (term._initialized) {
            return;
        }

        term._initialized = true;

        term.prompt = () => {
            term.write('\r\n$ ');
        };

        term.writeln('\x1b[32mHello.\x1b[0m \x1b[36mWelcomelo, even.\x1b[0m');
        prompt(term);


        term.history = [];
        term.hist_pos = 0;
        term.onData(e => {
            switch (e) {
                case '\u0003': // Ctrl+C
                    term.write('^C');
                    term.prompt();
                    break;
                case '\r': // Enter
                    runCommand(term, command);
                    if (command.length > 0) {
                        if (!(term.history.length > 0 && command == term.history[term.history.length - 1])) {
                            term.history.push(command);
                        }
                    }
                    term.hist_pos = term.history.length;
                    command = '';
                    break;
                case '\u007F': // Backspace (DEL)
                    // Do not delete the prompt
                    if (term._core.buffer.x > 2) {
                        term.write('\b \b');
                        if (command.length > 0) {
                            command = command.substr(0, command.length - 1);
                        }
                    }
                    break;
                case '[A':
                    if (term.hist_pos > 0) {
                        for (let i = 0; i < command.length; i++) {
                            term.write('\b \b');
                        }
                        term.hist_pos -= 1;
                        command = term.history[term.hist_pos];
                        term.write(command);
                    }
                    break;
                case '[B':
                    if (term.hist_pos < term.history.length) {
                        for (let i = 0; i < command.length; i++) {
                            term.write('\b \b');
                        }
                        term.hist_pos += 1;
                        if (term.hist_pos < term.history.length) {
                            command = term.history[term.hist_pos];
                        }
                        else { command = ''; }
                        term.write(command);
                    }
                    break;
                default: // Print all other characters for demo
                    if (e >= String.fromCharCode(0x20) && e <= String.fromCharCode(0x7E) || e >= '\u00a0') {
                        command += e;
                        term.write(e);
                    }
            }
        });

    }

    function prompt(term) {
        command = '';
        term.write('\r\n$ ');
    }

    var command = '';
    var commands = {
        help: {
            f: (_args) => {
                term.writeln([
                    '',
                    '',
                    'Try some of the commands below.',
                    '',
                    ...Object.keys(commands).map(e => `  ${e.padEnd(10)} ${commands[e].description}`)
                ].join('\r\n'));
            },
            description: 'Prints this help message',
        },
        history: {
            f: (_args) => {
                term.writeln('');
                term.writeln(term.history.join('\r\n'));
            },
            description: 'Prints command history',
        },
        echo: {
            f: (args) => {
                command_callback({ 'Echo': args });
            },
            description: "Contender for world's most contorted echo implementation",
        },
        hello: {
            f: (_args) => {
                command_callback('StartHello');
            },
            description: 'Start a cheerful Hello Server',
        },
        forth: {
            f: (args) => {
                command_callback({ 'Forth': args });
            },
            description: 'Execute a line of forth',
        }
    };

    function runCommand(term, text) {
        text = text.trim();
        if (text == "open the pod bay doors") {
            term.writeln(['', '', 'very funny.'].join('\r\n'));
        } else {
            const space_idx = text.indexOf(' ');
            let command, args;
            if (space_idx > 0) {
                [command, args] = [text.slice(0, space_idx), text.slice(space_idx + 1)];
            } else {
                [command, args] = [text, ""];
            }

            if (command.length > 0) {
                if (command in commands) {
                    commands[command].f(args);
                } else {
                    term.writeln(['', `${command}: command not found`].join('\r\n'));
                }
            }
        }

        term.prompt();
    }

    runFakeTerminal();

}
