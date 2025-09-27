#include "string_utils.hpp"
#include <algorithm>
#include <sstream>

namespace StringUtils {
    std::string to_upper(const std::string& str) {
        std::string result = str;
        std::transform(result.begin(), result.end(), result.begin(), ::toupper);
        return result;
    }
    
    std::string to_lower(const std::string& str) {
        std::string result = str;
        std::transform(result.begin(), result.end(), result.begin(), ::tolower);
        return result;
    }
    
    std::string reverse(const std::string& str) {
        std::string result = str;
        std::reverse(result.begin(), result.end());
        return result;
    }
    
    size_t word_count(const std::string& str) {
        std::istringstream iss(str);
        std::string word;
        size_t count = 0;
        while (iss >> word) {
            count++;
        }
        return count;
    }
}