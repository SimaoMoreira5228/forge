const std = @import("std");
const c = @cImport({
    @cInclude("raylib.h");
});

const SCREEN_WIDTH = 800;
const SCREEN_HEIGHT = 600;
const BALL_SPEED = 300.0;

const Ball = struct {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    radius: f32,

    fn init() Ball {
        return Ball{
            .x = SCREEN_WIDTH / 2.0,
            .y = SCREEN_HEIGHT / 2.0,
            .vx = BALL_SPEED,
            .vy = BALL_SPEED,
            .radius = 20.0,
        };
    }

    fn update(self: *Ball, dt: f32) void {
        self.x += self.vx * dt;
        self.y += self.vy * dt;

        if (self.x - self.radius <= 0 or self.x + self.radius >= SCREEN_WIDTH) {
            self.vx = -self.vx;
        }
        if (self.y - self.radius <= 0 or self.y + self.radius >= SCREEN_HEIGHT) {
            self.vy = -self.vy;
        }

        self.x = std.math.clamp(self.x, self.radius, SCREEN_WIDTH - self.radius);
        self.y = std.math.clamp(self.y, self.radius, SCREEN_HEIGHT - self.radius);
    }

    fn draw(self: *const Ball) void {
        c.DrawCircle(
            @as(c_int, @intFromFloat(self.x)),
            @as(c_int, @intFromFloat(self.y)),
            self.radius,
            c.RED,
        );
    }
};

pub fn main() void {
    c.InitWindow(SCREEN_WIDTH, SCREEN_HEIGHT, "Zig + Raylib - Bouncing Ball");
    defer c.CloseWindow();

    c.SetTargetFPS(60);

    var ball = Ball.init();
    var frame_count: u32 = 0;

    std.debug.print("Starting Zig + Raylib game!\n", .{});
    std.debug.print("Press SPACE to reset ball position\n", .{});
    std.debug.print("Press ESC to exit\n\n", .{});

    while (!c.WindowShouldClose()) {
        const dt = c.GetFrameTime();
        ball.update(dt);

        if (c.IsKeyPressed(c.KEY_SPACE)) {
            ball = Ball.init();
        }

        frame_count += 1;

        c.BeginDrawing();
        defer c.EndDrawing();

        c.ClearBackground(c.RAYWHITE);

        ball.draw();

        c.DrawText(
            "Zig + Raylib Example",
            10,
            10,
            20,
            c.DARKGRAY,
        );

        c.DrawText(
            "Press SPACE to reset",
            10,
            40,
            15,
            c.GRAY,
        );

        c.DrawFPS(SCREEN_WIDTH - 100, 10);

        const frame_text = std.fmt.allocPrint(
            std.heap.c_allocator,
            "Frame: {d}\x00",
            .{frame_count},
        ) catch "Frame: ???\x00";
        defer if (@intFromPtr(frame_text.ptr) != @intFromPtr("Frame: ???\x00")) {
            std.heap.c_allocator.free(frame_text);
        };

        c.DrawText(
            @ptrCast(frame_text.ptr),
            10,
            SCREEN_HEIGHT - 30,
            15,
            c.DARKGRAY,
        );
    }

    std.debug.print("\nGame closed. Frames rendered: {d}\n", .{frame_count});
}
