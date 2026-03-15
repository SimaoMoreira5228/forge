#include <gtest/gtest.h>
import math_utils;

TEST(MathUtilsTest, Add) {
    EXPECT_EQ(add(2, 3), 5);
    EXPECT_EQ(add(-1, 1), 0);
    EXPECT_EQ(add(0, 0), 0);
}

TEST(MathUtilsTest, Multiply) {
    EXPECT_EQ(multiply(2, 3), 6);
    EXPECT_EQ(multiply(-2, 3), -6);
    EXPECT_EQ(multiply(0, 5), 0);
}

TEST(MathUtilsTest, AddMultiple) {
    EXPECT_EQ(add(add(1, 2), 3), 6);
    EXPECT_EQ(add(add(10, 20), add(30, 40)), 100);
}

int main(int argc, char **argv) {
    ::testing::InitGoogleTest(&argc, argv);
    return RUN_ALL_TESTS();
}
