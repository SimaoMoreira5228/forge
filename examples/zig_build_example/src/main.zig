const std = @import("std");
const lib = @import("lib.zig");

pub fn main() void {
    std.debug.print("Hello from Zig Build!\n", .{});
    std.debug.print("====================\n\n", .{});

    std.debug.print("Fibonacci sequence:\n", .{});
    var i: u32 = 0;
    while (i <= 15) : (i += 1) {
        std.debug.print("fib({d:2}) = {d}\n", .{ i, lib.fibonacci(i) });
    }
}
