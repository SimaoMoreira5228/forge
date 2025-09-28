cmake_minimum_required(VERSION 3.10)
project(HelloCMake C)

set(CMAKE_C_STANDARD 11)

# Simple library
add_library(greetings STATIC
    src/greetings.c
)

target_include_directories(greetings PUBLIC include)

# Executable
add_executable(hello src/main.c)
target_link_libraries(hello greetings)

# Installation
install(TARGETS hello DESTINATION bin)
install(TARGETS greetings DESTINATION lib)
install(FILES include/greetings.h DESTINATION include)
