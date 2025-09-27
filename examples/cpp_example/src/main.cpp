#include <iostream>
#include <string>
#include "string_utils.hpp"

int main() {
    std::string text = "Hello, World! This is a C++ example.";
    
    std::cout << "Text Processor Demo\n";
    std::cout << "Original: " << text << "\n";
    std::cout << "Uppercase: " << StringUtils::to_upper(text) << "\n";
    std::cout << "Lowercase: " << StringUtils::to_lower(text) << "\n";
    std::cout << "Reversed: " << StringUtils::reverse(text) << "\n";
    std::cout << "Word count: " << StringUtils::word_count(text) << "\n";
    
    return 0;
}