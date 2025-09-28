#ifndef STRING_UTILS_HPP
#define STRING_UTILS_HPP

#include <string>

namespace StringUtils {
    std::string to_upper(const std::string& str);
    std::string to_lower(const std::string& str);
    std::string reverse(const std::string& str);
    size_t word_count(const std::string& str);
}

#endif