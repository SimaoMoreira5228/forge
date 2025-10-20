const std = @import("std");

pub fn greet(name: []const u8, writer: anytype) !void {
    try writer.print("Hello, {s}!\n", .{name});
}

pub fn fibonacci(n: u32) u64 {
    if (n <= 1) return n;

    var prev: u64 = 0;
    var curr: u64 = 1;
    var i: u32 = 2;

    while (i <= n) : (i += 1) {
        const next = prev + curr;
        prev = curr;
        curr = next;
    }

    return curr;
}

test "fibonacci" {
    try std.testing.expectEqual(@as(u64, 0), fibonacci(0));
    try std.testing.expectEqual(@as(u64, 1), fibonacci(1));
    try std.testing.expectEqual(@as(u64, 55), fibonacci(10));
}
