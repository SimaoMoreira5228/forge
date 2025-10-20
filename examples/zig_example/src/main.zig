const std = @import("std");

pub fn main() void {
    std.debug.print("Simple Zig Calculator\n", .{});
    std.debug.print("====================\n\n", .{});

    std.debug.print("Simple Zig Calculator\n", .{});
    std.debug.print("====================\n\n", .{});

    const a: i32 = 10;
    const b: i32 = 5;

    std.debug.print("Basic Math Operations:\n", .{});
    std.debug.print("{d} + {d} = {d}\n", .{ a, b, a + b });
    std.debug.print("{d} * {d} = {d}\n", .{ a, b, a * b });

    std.debug.print("\nFactorials:\n", .{});
    var i: u32 = 0;
    while (i <= 10) : (i += 1) {
        const fact = computeFactorial(i);
        std.debug.print("{d}! = {d}\n", .{ i, fact });
    }
}

fn computeFactorial(n: u32) u64 {
    if (n == 0) return 1;
    var result: u64 = 1;
    var i: u32 = 1;
    while (i <= n) : (i += 1) {
        result *= i;
    }
    return result;
}
