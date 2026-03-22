import std.stdio;

extern (C) {
    void hello_from_c();
}

extern (C++) {
    struct HelloCpp {
        void print();
    }
}

void main() {
    writeln("Starting D interop example...");
    
    hello_from_c();
    
    HelloCpp cpp;
    cpp.print();
    
    writeln("Done!");
}
