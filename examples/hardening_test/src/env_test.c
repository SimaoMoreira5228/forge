#include <stdio.h>
#include <stdlib.h>

int main() {
    char* home = getenv("HOME");
    if (home) {
        printf("VIOLATION: HOME is set to: %s\n", home);
        return 1; 
    } else {
        printf("SUCCESS: HOME is NOT set\n");
        return 0;
    }
}
