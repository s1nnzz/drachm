---
title: A window with SDL3
description: An interactive window in safe Drachm over the SDL3 C library, with the unsafe part kept in one wrapper module.
---

# A window with SDL3

`examples/sdl3-window/` opens a window, draws into it, and takes keyboard and
mouse input. It is the [C imports](guide-c-imports.md) guide as one program:
the C library (SDL3) is called from one module, `screen.drh`, which gives the
rest of the program a `Window` with safe methods and a Drachm enum of events.
`main.drh` has no `unsafe`, no C type and no pointer in it.

It needs SDL3 installed (headers and library) and libclang, like any C import.
Run it with:

```text
$ drachm run examples/sdl3-window
$ drachm run examples/sdl3-window -O        # the same program through C
```

| Input | Effect |
|---|---|
| WASD or the arrow keys | move the square |
| the mouse | a crosshair and a fading trail follow it |
| a click | pins a line from the square to the mouse |
| Space | centres the square and removes the pin |
| Escape, or closing the window | quits |

Without a display, SDL's dummy driver runs the same program for a number of
frames: `SDL_VIDEO_DRIVER=dummy WINDOW_FRAMES=120 drachm run examples/sdl3-window`.
`cargo test` does exactly that when SDL3 is installed.

## The manifest

```toml
# An SDL3 window: a safe `Window` over the C library, used from a program with
# no unsafe in it. `drachm run examples/sdl3-window` needs SDL3 installed.
[project]
name = "sdl3window"

[native]
libraries = ["SDL3"]
unsafe = true   # screen.drh calls C and reads the SDL_Event union
```

`libraries` links `libSDL3`, and `unsafe = true` lets the files of this project
call C. The header, `SDL3/SDL.h`, is found on the system's include path.

## The wrapper, `screen.drh`

```drachm
// A safe window over SDL3. Everything unsafe in the program lives in this
// file: the C calls, the raw pointers and the SDL_Event union. The rest of the
// program (main.drh) sees a Window with methods and a Drachm enum of events.

in * from c."SDL3/SDL.h" as sdl;

// The keys the program cares about. WASD and the arrows are the same keys.
out enum Key { Left, Right, Up, Down, Escape, Space, Other }

// What happened since the last call to `poll`.
out enum Event {
    Quit,
    Pressed(Key key),
    Released(Key key),
    MouseMoved(float32 x, float32 y),
    MouseDown(float32 x, float32 y),
    Idle,
}

out failure ScreenError {
    InitFailed(string reason) f"could not start SDL: ${reason}",
    WindowFailed(string reason) f"could not open a window: ${reason}",
}

// What SDL last complained about, as a Drachm string.
string reason() {
    unsafe {
        foreign *uint8 text = sdl::SDL_GetError();
        return (text::text() maybe none) maybe "unknown error";
    }
}

// An open window and the renderer that draws into it. `drop()` closes both,
// and shuts SDL down, whichever way the scope that owns the Window ends.
out struct Window {
    foreign *sdl::SDL_Window window;
    foreign *sdl::SDL_Renderer renderer;
    sint32 width;
    sint32 height;

    // Fill the whole window with one colour.
    none clear(uint8 r, uint8 g, uint8 b) {
        unsafe {
            sdl::SDL_SetRenderDrawColor(self.renderer, r as c::uchar, g as c::uchar, b as c::uchar, 255);
            sdl::SDL_RenderClear(self.renderer);
        }
    }

    // A filled rectangle. Positions and sizes are in pixels.
    none fillRect(float32 x, float32 y, float32 w, float32 h, uint8 r, uint8 g, uint8 b) {
        unsafe {
            mut sdl::SDL_FRect rect = x as c::float, y as c::float, w as c::float, h as c::float;
            sdl::SDL_SetRenderDrawColor(self.renderer, r as c::uchar, g as c::uchar, b as c::uchar, 255);
            sdl::SDL_RenderFillRect(self.renderer, (address of rect) as foreign *sdl::SDL_FRect);
        }
    }

    // A line from one point to another.
    none line(float32 x1, float32 y1, float32 x2, float32 y2, uint8 r, uint8 g, uint8 b) {
        unsafe {
            sdl::SDL_SetRenderDrawColor(self.renderer, r as c::uchar, g as c::uchar, b as c::uchar, 255);
            sdl::SDL_RenderLine(self.renderer, x1 as c::float, y1 as c::float, x2 as c::float, y2 as c::float);
        }
    }

    // Show what was drawn. With vsync on, this waits for the next refresh.
    none present() {
        unsafe { sdl::SDL_RenderPresent(self.renderer); }
    }

    // The next thing that happened, or Idle when nothing is waiting.
    Event poll() {
        // SDL fills the union with whichever member the event is; `type` says which.
        mut sdl::SDL_Event ev = new sdl::SDL_Event of ();
        unsafe {
            bool got = sdl::SDL_PollEvent((address of ev) as foreign *sdl::SDL_Event) as bool;
            if (!got) {
                return Idle;
            }
            uint32 kind = ev.type as uint32;
            if (kind == sdl::SDL_EVENT_QUIT as uint32) { return Quit; }
            if (kind == sdl::SDL_EVENT_KEY_DOWN as uint32) { return Pressed(keyOf(ev.key.key as uint32)); }
            if (kind == sdl::SDL_EVENT_KEY_UP as uint32) { return Released(keyOf(ev.key.key as uint32)); }
            if (kind == sdl::SDL_EVENT_MOUSE_MOTION as uint32) { return MouseMoved(inside(ev.motion.x as float32, self.width), inside(ev.motion.y as float32, self.height)); }
            if (kind == sdl::SDL_EVENT_MOUSE_BUTTON_DOWN as uint32) { return MouseDown(ev.button.x as float32, ev.button.y as float32); }
        }
        // Some other event (focus, window moved, ...): nothing this program uses.
        return Idle;
    }

    none drop() {
        unsafe {
            sdl::SDL_DestroyRenderer(self.renderer);
            sdl::SDL_DestroyWindow(self.window);
            sdl::SDL_Quit();
        }
    }
}

// Wait, in milliseconds.
out none wait(uint64 ms) {
    unsafe { sdl::SDL_Delay(ms::to<c::uint>()); }
}

// Milliseconds since SDL started.
out uint64 ticks() {
    unsafe { return sdl::SDL_GetTicks() as uint64; }
}

// A mouse coordinate kept inside the window.
float32 inside(float32 v, sint32 limit) {
    if (v < 0.0) { return 0.0; }
    if (v > limit::to<float32>()) { return limit::to<float32>(); }
    return v;
}

Key keyOf(uint32 code) {
    if (code == sdl::SDLK_LEFT || code == sdl::SDLK_A) { return Left; }
    if (code == sdl::SDLK_RIGHT || code == sdl::SDLK_D) { return Right; }
    if (code == sdl::SDLK_UP || code == sdl::SDLK_W) { return Up; }
    if (code == sdl::SDLK_DOWN || code == sdl::SDLK_S) { return Down; }
    if (code == sdl::SDLK_ESCAPE) { return Escape; }
    if (code == sdl::SDLK_SPACE) { return Space; }
    return Other;
}

// Start SDL and open a window with a renderer. Failures come back as ScreenError.
out Window openWindow(string title, sint32 width, sint32 height) fails ScreenError {
    unsafe {
        bool started = sdl::SDL_Init(sdl::SDL_INIT_VIDEO) as bool;
        if (!started) {
            fail InitFailed(reason());
        }
        foreign *sdl::SDL_Window window = sdl::SDL_CreateWindow(&title as c string, width as c::int, height as c::int, 0);
        if (window == none) {
            string why = reason();
            sdl::SDL_Quit();
            fail WindowFailed(why);
        }
        foreign *sdl::SDL_Renderer renderer = sdl::SDL_CreateRenderer(window, none);
        if (renderer == none) {
            string why = reason();
            sdl::SDL_DestroyWindow(window);
            sdl::SDL_Quit();
            fail WindowFailed(why);
        }
        sdl::SDL_SetRenderVSync(renderer, 1);
        return new Window of (window: window, renderer: renderer, width: width, height: height);
    }
}
```

Things worth noticing:

- **The union.** SDL hands every event to `SDL_PollEvent` as an `SDL_Event`,
  a C union of about 50 structs. `new sdl::SDL_Event of ()` makes it (zero
  bytes), `(address of ev) as foreign *sdl::SDL_Event` gives C its address, and
  `ev.type` says which member is valid. Reading `ev.key.key` or
  `ev.motion.x` is `unsafe` ([E0336](errors.md)): the `if` on `kind` (the event type) above each
  read is what makes it sound.
- **`SDL_FRect` by pointer.** `SDL_RenderFillRect` takes a pointer to a struct
  of four floats; `fillRect` builds the struct from plain numbers and hands over
  its address for that one call.
- **`drop()` releases the C handles.** The renderer and the window are destroyed
  and SDL is shut down when the `Window` goes out of scope, including when
  `main` fails on the way out. No inference: the wrapper pairs the calls.
- **Errors become failures.** C reports failure with a `false` result and a
  message from `SDL_GetError()`; `openWindow` turns that into a `ScreenError`
  that `main` passes up.
- **Events become an enum.** `Event` is an ordinary Drachm `enum`, so `main`
  matches on it like any other.

## The program, `main.drh`

```drachm
// A window with SDL3, written in safe Drachm. All the C lives in screen.drh;
// this file has no `unsafe`, no C types and no pointers.
//
//   WASD or the arrow keys    move the square
//   the mouse                 leaves a trail and a crosshair
//   a click                   pins a line from the square to the mouse
//   Space                     centres the square and removes the pin
//   Escape or closing         quits
//
// WINDOW_FRAMES=N quits after N frames (for a run without a display:
// `SDL_VIDEO_DRIVER=dummy WINDOW_FRAMES=120 drachm run examples/sdl3-window`).

in io;
in os;
in screen;

out uint64 FRAME_MS = 16;
out sint32 WIDTH = 640;
out sint32 HEIGHT = 480;
out uint32 TRAIL = 24;

// Which movement keys are held right now.
struct Held {
public:
    bool left;
    bool right;
    bool up;
    bool down;

    none set(screen::Key key, bool down_now) {
        match (key) {
            case Left { self.left = down_now; }
            case Right { self.right = down_now; }
            case Up { self.up = down_now; }
            case Down { self.down = down_now; }
            case _ { }
        }
    }
}

// How many frames to run: WINDOW_FRAMES, or until the window is closed.
uint32 frameLimit() {
    string? text = os::getEnv("WINDOW_FRAMES");
    string value = text maybe "0";
    return value::to<uint32>() maybe 0;
}

none main() fails screen::ScreenError {
    screen::Window win = screen::openWindow("Drachm and SDL3", WIDTH, HEIGHT) fails;
    uint32 limit = frameLimit();

    mut float32 x = 300.0;
    mut float32 y = 220.0;
    mut Held held = false, false, false, false;
    mut float32 mouseX = 320.0;
    mut float32 mouseY = 240.0;
    // The last TRAIL mouse positions, a ring that starts at the middle.
    mut float32[?] trailX = [];
    mut float32[?] trailY = [];
    mut uint32 filled = 0;
    while (filled < TRAIL) {
        trailX::push(320.0);
        trailY::push(240.0);
        filled += 1;
    }
    mut uint32 slot = 0;
    mut bool pinned = false;
    mut float32 pinX = 0.0;
    mut float32 pinY = 0.0;
    mut bool running = true;
    mut uint32 frame = 0;
    mut uint32 clicks = 0;

    while (running) {
        uint64 started = screen::ticks();

        // Everything that happened since the last frame.
        mut bool pending = true;
        while (pending) {
            match (win::poll()) {
                case Quit { running = false; }
                case Pressed(Escape) { running = false; }
                case Pressed(Space) {
                    x = 300.0;
                    y = 220.0;
                    pinned = false;
                }
                case Pressed(screen::Key key) { held::set(key, true); }
                case Released(screen::Key key) { held::set(key, false); }
                case MouseMoved(float32 mx, float32 my) {
                    mouseX = mx;
                    mouseY = my;
                    trailX[slot] = mx;
                    trailY[slot] = my;
                    slot = (slot + 1) % TRAIL;
                }
                case MouseDown(float32 mx, float32 my) {
                    pinned = true;
                    pinX = mx;
                    pinY = my;
                    clicks += 1;
                }
                case Idle { pending = false; }
            }
        }

        // Move the square and keep it inside the window.
        if (held.left) { x -= 5.0; }
        if (held.right) { x += 5.0; }
        if (held.up) { y -= 5.0; }
        if (held.down) { y += 5.0; }
        if (x < 0.0) { x = 0.0; }
        if (x > 600.0) { x = 600.0; }
        if (y < 0.0) { y = 0.0; }
        if (y > 440.0) { y = 440.0; }

        // Draw.
        win::clear(18, 22, 40);
        mut uint32 i = 0;
        while (i < TRAIL) {
            // Older points are dimmer and smaller.
            uint32 age = (slot + i) % TRAIL;
            uint8 shade = (40 + i * 8)::to<uint8>();
            float32 size = 2.0 + (i::to<float32>() / 4.0);
            win::fillRect(trailX[age], trailY[age], size, size, shade, shade, 255);
            i += 1;
        }
        win::line(mouseX - 12.0, mouseY, mouseX + 12.0, mouseY, 120, 220, 160);
        win::line(mouseX, mouseY - 12.0, mouseX, mouseY + 12.0, 120, 220, 160);
        if (pinned) {
            win::line(x + 20.0, y + 20.0, pinX, pinY, 255, 220, 80);
            win::fillRect(pinX - 4.0, pinY - 4.0, 8.0, 8.0, 255, 220, 80);
        }
        // The square pulses between two oranges.
        mut uint32 pulse = frame % 120;
        if (pulse >= 60) { pulse = 119 - pulse; }
        uint8 glow = (140 + pulse)::to<uint8>();
        win::fillRect(x, y, 40.0, 40.0, 255, glow, 60);
        win::present();

        frame += 1;
        if (limit > 0 && frame >= limit) { running = false; }

        // Keep to about 60 frames a second even where vsync does not wait.
        uint64 spent = screen::ticks() - started;
        if (spent < FRAME_MS) { screen::wait(FRAME_MS - spent); }
    }
    io::outln(f"closed after ${frame} frames, ${clicks} clicks");
}
```
