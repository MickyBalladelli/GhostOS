#include "ghostos/path_pattern.h"

_Static_assert(sizeof(ghostos_pattern_error) == sizeof(uint32_t), "pattern error ABI");

static ghostos_pattern_error class_end(const uint8_t *pattern, size_t length,
    size_t start, size_t *end) {
    size_t index = start + 1, members = 0;
    if (index < length && (pattern[index] == '!' || pattern[index] == '^')) ++index;
    while (index < length) {
        if (pattern[index] == '\\') {
            if (++index == length) return GHOSTOS_PATTERN_TRAILING_ESCAPE;
            ++members;
            ++index;
        } else if (pattern[index] == ']') {
            if (!members) return GHOSTOS_PATTERN_EMPTY_CLASS;
            *end = index;
            return GHOSTOS_PATTERN_OK;
        } else {
            ++members;
            ++index;
        }
    }
    return GHOSTOS_PATTERN_UNTERMINATED_CLASS;
}

static bool match_class(const uint8_t *pattern, size_t length,
    const uint8_t *candidate, size_t candidate_length,
    bool *matched, size_t *consumed) {
    if (!length || pattern[0] != '[' || !candidate_length || candidate[0] == '/') return false;
    size_t end;
    if (class_end(pattern, length, 0, &end) != GHOSTOS_PATTERN_OK) return false;
    size_t index = 1;
    bool negated = pattern[index] == '!' || pattern[index] == '^';
    if (negated) ++index;
    bool found = false;
    while (index < end) {
        if (pattern[index] == '\\') ++index;
        uint8_t left = pattern[index++];
        if (index + 1 < end && pattern[index] == '-' && pattern[index + 1] != ']') {
            ++index;
            if (pattern[index] == '\\') ++index;
            uint8_t right = pattern[index++];
            if (left > right) return false;
            found |= left <= candidate[0] && candidate[0] <= right;
        } else found |= left == candidate[0];
    }
    *matched = negated ? !found : found;
    *consumed = end + 1;
    return true;
}

ghostos_pattern_error ghostos_pattern_parse(const uint8_t *pattern,
    size_t length, bool *magic) {
    *magic = false;
    if (!length) return GHOSTOS_PATTERN_EMPTY;
    if (length > GHOSTOS_MAX_PATTERN_BYTES ||
        (length != 1 && pattern[length - 1] == '/')) return GHOSTOS_PATTERN_INVALID_PATH;
    for (size_t i = 0; i < length; ++i) {
        if (!pattern[i] || (i + 1 < length && pattern[i] == '/' && pattern[i + 1] == '/'))
            return GHOSTOS_PATTERN_INVALID_PATH;
    }
    size_t index = 0;
    while (index < length) {
        switch (pattern[index]) {
            case '\\':
                if (++index == length) return GHOSTOS_PATTERN_TRAILING_ESCAPE;
                ++index;
                break;
            case '*': case '?':
                *magic = true;
                ++index;
                break;
            case '[': {
                size_t end, consumed;
                ghostos_pattern_error error = class_end(pattern, length, index, &end);
                if (error != GHOSTOS_PATTERN_OK) return error;
                const uint8_t zero = 0;
                bool matched;
                if (!match_class(pattern + index, length - index, &zero, 1, &matched, &consumed))
                    return GHOSTOS_PATTERN_INVALID_RANGE;
                *magic = true;
                index = end + 1;
                break;
            }
            default: ++index; break;
        }
    }
    return GHOSTOS_PATTERN_OK;
}

static size_t character_width(const uint8_t *value, size_t length) {
    size_t width = value[0] < 0x80 ? 1 : value[0] < 0xe0 ? 2 : value[0] < 0xf0 ? 3 : 4;
    return width < length ? width : length;
}

bool ghostos_pattern_matches(const uint8_t *pattern, size_t length,
    const uint8_t *candidate, size_t candidate_length) {
    if (!length) return !candidate_length;
    switch (pattern[0]) {
        case '\\':
            return length > 1 && candidate_length && pattern[1] == candidate[0] &&
                ghostos_pattern_matches(pattern + 2, length - 2, candidate + 1, candidate_length - 1);
        case '*': {
            /* Iterate star consumption to avoid recursion proportional to path length. */
            for (;;) {
                if (ghostos_pattern_matches(pattern + 1, length - 1, candidate, candidate_length)) return true;
                if (!candidate_length || candidate[0] == '/') return false;
                size_t width = character_width(candidate, candidate_length);
                candidate += width;
                candidate_length -= width;
            }
        }
        case '?': {
            if (!candidate_length || candidate[0] == '/') return false;
            size_t width = character_width(candidate, candidate_length);
            return ghostos_pattern_matches(pattern + 1, length - 1, candidate + width, candidate_length - width);
        }
        case '[': {
            bool matched;
            size_t consumed;
            if (!match_class(pattern, length, candidate, candidate_length, &matched, &consumed) || !matched) return false;
            size_t width = character_width(candidate, candidate_length);
            return ghostos_pattern_matches(pattern + consumed, length - consumed, candidate + width, candidate_length - width);
        }
        default:
            return candidate_length && pattern[0] == candidate[0] &&
                ghostos_pattern_matches(pattern + 1, length - 1, candidate + 1, candidate_length - 1);
    }
}

ghostos_pattern_error ghostos_pattern_unescape(const uint8_t *pattern,
    size_t length, uint8_t *output, size_t capacity, size_t *written) {
    *written = 0;
    for (size_t index = 0; index < length; ++index) {
        if (pattern[index] == '\\' && ++index == length) return GHOSTOS_PATTERN_TRAILING_ESCAPE;
        if (*written == capacity) return GHOSTOS_PATTERN_TOO_LONG;
        output[(*written)++] = pattern[index];
    }
    return GHOSTOS_PATTERN_OK;
}
