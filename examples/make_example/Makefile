CC = gcc
CFLAGS = -Wall -Wextra -O2 -Iinclude
AR = ar
ARFLAGS = rcs

# Directories
SRC_DIR = src
BUILD_DIR = build
OBJ_DIR = $(BUILD_DIR)/obj

# Source files
LIB_SRCS = $(SRC_DIR)/math_utils.c
MAIN_SRC = $(SRC_DIR)/main.c

# Object files
LIB_OBJS = $(OBJ_DIR)/math_utils.o
MAIN_OBJ = $(OBJ_DIR)/main.o

# Outputs
LIB = $(BUILD_DIR)/libmath.a
EXEC = $(BUILD_DIR)/calculator

.PHONY: all clean

all: $(LIB) $(EXEC)

$(OBJ_DIR):
	mkdir -p $(OBJ_DIR)

$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

# Library
$(LIB): $(LIB_OBJS) | $(BUILD_DIR)
	$(AR) $(ARFLAGS) $@ $^

# Executable
$(EXEC): $(MAIN_OBJ) $(LIB) | $(BUILD_DIR)
	$(CC) $(CFLAGS) -o $@ $^

# Object files
$(OBJ_DIR)/%.o: $(SRC_DIR)/%.c | $(OBJ_DIR)
	$(CC) $(CFLAGS) -c $< -o $@

clean:
	rm -rf $(BUILD_DIR)
