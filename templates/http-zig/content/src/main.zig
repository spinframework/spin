const std = @import("std");

pub fn main() !void {
    const stdout = std.io.getStdOutWriter();
    try stdout.print("content-type: text/plain\n\n", .{});
    try stdout.print("Hello World!\n", .{});
}
