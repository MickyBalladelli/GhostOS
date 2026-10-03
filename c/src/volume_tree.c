#include "ghostos/volume_tree.h"
static uint32_t load32(const uint8_t *bytes) {
    return (uint32_t)bytes[0] | ((uint32_t)bytes[1] << 8) |
        ((uint32_t)bytes[2] << 16) | ((uint32_t)bytes[3] << 24);
}
static void store32(uint8_t *bytes, uint32_t value) {
    size_t i;
    for (i = 0; i < 4; ++i) bytes[i] = (uint8_t)(value >> (8 * i));
}
static ghostos_volume_tree_key key_of(const ghostos_volume_record *record) {
    ghostos_volume_tree_key key = {0};
    size_t i;
    key.name_length = record->name_length;
    key.version = record->version;
    for (i = 0; i < GHOSTOS_VOLUME_NAME; ++i) key.name[i] = record->name[i];
    return key;
}
static int compare(const ghostos_volume_tree_key *left, const ghostos_volume_tree_key *right) {
    size_t i, length = left->name_length < right->name_length ? left->name_length : right->name_length;
    for (i = 0; i < length; ++i) if (left->name[i] != right->name[i]) return left->name[i] < right->name[i] ? -1 : 1;
    if (left->name_length != right->name_length) return left->name_length < right->name_length ? -1 : 1;
    if (left->version == right->version) return 0;
    return left->version < right->version ? -1 : 1;
}
static bool valid_utf8(const uint8_t *bytes, size_t length) {
    size_t i = 0;
    while (i < length) {
        uint8_t first = bytes[i++];
        uint32_t value, minimum;
        size_t remaining;
        if (first < 0x80) continue;
        if (first >= 0xc2 && first <= 0xdf) { value = first & 0x1f; minimum = 0x80; remaining = 1; }
        else if (first >= 0xe0 && first <= 0xef) { value = first & 0x0f; minimum = 0x800; remaining = 2; }
        else if (first >= 0xf0 && first <= 0xf4) { value = first & 0x07; minimum = 0x10000; remaining = 3; }
        else return false;
        if (remaining > length - i) return false;
        while (remaining) {
            uint8_t next = bytes[i++];
            remaining -= 1;
            if ((next & 0xc0) != 0x80) return false;
            value = (value << 6) | (next & 0x3f);
        }
        if (value < minimum || value > 0x10ffff || (value >= 0xd800 && value <= 0xdfff)) return false;
    }
    return true;
}
static bool same_key(const ghostos_volume_tree_key *left, const ghostos_volume_tree_key *right) {
    size_t i;
    if (left->name_length != right->name_length || left->version != right->version) return false;
    for (i = 0; i < GHOSTOS_VOLUME_NAME; ++i) if (left->name[i] != right->name[i]) return false;
    return true;
}
static int decode_record(const uint8_t *bytes, size_t length,
    ghostos_volume_record *record, size_t *consumed) {
    uint8_t encoded[GHOSTOS_VOLUME_RECORD];
    size_t i;
    uint16_t name_length;
    int status;
    if (length < GHOSTOS_VOLUME_RECORD) return 1;
    name_length = (uint16_t)bytes[0] | ((uint16_t)bytes[1] << 8);
    if (name_length > GHOSTOS_VOLUME_NAME || !valid_utf8(bytes + 2, name_length)) return 1;
    for (i = 0; i < GHOSTOS_VOLUME_RECORD; ++i) encoded[i] = bytes[i];
    /* The standalone record codec rejects NUL names. Tree decoding follows the
       disk decoder, which accepts valid UTF-8 NUL and retains all name bytes. */
    for (i = 0; i < name_length; ++i) if (!encoded[2 + i]) encoded[2 + i] = 1;
    status = ghostos_volume_decode_record(encoded, sizeof encoded, record, consumed);
    if (status) return 1;
    for (i = 0; i < GHOSTOS_VOLUME_NAME; ++i) record->name[i] = bytes[2 + i];
    return 0;
}
int ghostos_volume_tree_decode(const uint8_t *block, size_t capacity,
    uint8_t *payload, size_t payload_capacity, ghostos_volume_tree_node *node) {
    size_t length, cursor = 4, i, consumed;
    uint8_t kind;
    int status = ghostos_volume_decode_tree(block, capacity, &kind, payload, payload_capacity, &length);
    if (status) return status;
    if (length < 4 || payload[0] > GHOSTOS_VOLUME_TREE_KEYS) return 1;
    *node = (ghostos_volume_tree_node){0};
    node->kind = kind;
    node->length = payload[0];
    if (kind == 1) {
        for (i = 0; i < GHOSTOS_VOLUME_TREE_KEYS; ++i) {
            status = decode_record(payload + cursor, length - cursor,
                &node->entries.records[i], &consumed);
            if (status) return 1;
            cursor += consumed;
        }
    } else {
        for (i = 0; i < GHOSTOS_VOLUME_TREE_KEYS; ++i) {
            ghostos_volume_tree_key *key = &node->entries.branch.keys[i];
            size_t byte;
            if (cursor > length || length - cursor < 2 + GHOSTOS_VOLUME_NAME + 4) return 1;
            key->name_length = (uint16_t)payload[cursor] | ((uint16_t)payload[cursor + 1] << 8);
            if (key->name_length > GHOSTOS_VOLUME_NAME) return 1;
            if (!valid_utf8(payload + cursor + 2, key->name_length)) return 1;
            for (byte = 0; byte < GHOSTOS_VOLUME_NAME; ++byte)
                key->name[byte] = payload[cursor + 2 + byte];
            key->version = load32(payload + cursor + 2 + GHOSTOS_VOLUME_NAME);
            cursor += 2 + GHOSTOS_VOLUME_NAME + 4;
        }
        if (cursor > length || length - cursor < 4 * (GHOSTOS_VOLUME_TREE_KEYS + 1)) return 1;
        for (i = 0; i <= GHOSTOS_VOLUME_TREE_KEYS; ++i)
            node->entries.branch.children[i] = load32(payload + cursor + 4 * i);
    }
    return 0;
}
int ghostos_volume_tree_encode(const ghostos_volume_tree_node *node,
    uint8_t *payload, size_t payload_capacity, uint8_t *block, size_t capacity) {
    size_t cursor = 4, i, written;
    if (node->length > GHOSTOS_VOLUME_TREE_KEYS || (node->kind != 1 && node->kind != 2)) return 1;
    if (payload_capacity < GHOSTOS_VOLUME_DATA) return 2;
    payload[0] = node->length;
    payload[1] = payload[2] = payload[3] = 0;
    if (node->kind == 1) {
        for (i = 0; i < GHOSTOS_VOLUME_TREE_KEYS; ++i) {
            int status = ghostos_volume_encode_record(&node->entries.records[i],
                payload + cursor, payload_capacity - cursor, &written);
            if (status) return status;
            for (size_t byte = 0; byte < GHOSTOS_VOLUME_NAME; ++byte)
                payload[cursor + 2 + byte] = node->entries.records[i].name[byte];
            cursor += written;
        }
    } else {
        for (i = 0; i < GHOSTOS_VOLUME_TREE_KEYS; ++i) {
            const ghostos_volume_tree_key *key = &node->entries.branch.keys[i];
            size_t byte;
            if (key->name_length > GHOSTOS_VOLUME_NAME) return 1;
            payload[cursor] = (uint8_t)key->name_length;
            payload[cursor + 1] = (uint8_t)(key->name_length >> 8);
            for (byte = 0; byte < GHOSTOS_VOLUME_NAME; ++byte)
                payload[cursor + 2 + byte] = key->name[byte];
            store32(payload + cursor + 2 + GHOSTOS_VOLUME_NAME, key->version);
            cursor += 2 + GHOSTOS_VOLUME_NAME + 4;
        }
        for (i = 0; i <= GHOSTOS_VOLUME_TREE_KEYS; ++i) {
            store32(payload + cursor, node->entries.branch.children[i]);
            cursor += 4;
        }
    }
    return ghostos_volume_encode_tree(node->kind, payload, cursor, block, capacity);
}
static int valid_tree(const ghostos_volume_tree *tree, const ghostos_volume_record *record) {
    if (!tree || !record || record->name_length > GHOSTOS_VOLUME_NAME ||
        !valid_utf8(record->name, record->name_length) ||
        tree->block_count > UINT32_MAX || tree->block_count > SIZE_MAX / GHOSTOS_VOLUME_BLOCK ||
        (tree->block_count && (!tree->blocks || !tree->kinds))) return 1;
    if (!tree->payload || tree->payload_capacity < GHOSTOS_VOLUME_DATA ||
        (tree->frame_capacity && !tree->frames)) return 2;
    return 0;
}
static int read_node(ghostos_volume_tree *tree, uint32_t id, ghostos_volume_tree_node *node) {
    if (!id || id > tree->block_count || !tree->kinds[id - 1] || tree->kinds[id - 1] > 2) return 1;
    return ghostos_volume_tree_decode(tree->blocks + (size_t)(id - 1) * GHOSTOS_VOLUME_BLOCK,
        GHOSTOS_VOLUME_BLOCK, tree->payload, tree->payload_capacity, node);
}
static int allocate(ghostos_volume_tree *tree, const ghostos_volume_tree_node *node, uint32_t *id) {
    size_t i;
    int status;
    for (i = 0; i < tree->block_count; ++i) {
        if (tree->kinds[i]) continue;
        status = ghostos_volume_tree_encode(node, tree->payload, tree->payload_capacity,
            tree->blocks + i * GHOSTOS_VOLUME_BLOCK, GHOSTOS_VOLUME_BLOCK);
        if (status) return status;
        tree->kinds[i] = node->kind;
        *id = (uint32_t)(i + 1);
        return 0;
    }
    return 3;
}
typedef struct {
    uint32_t left, right;
    bool split;
    ghostos_volume_tree_key separator;
} inserted;
static int insert_leaf(ghostos_volume_tree *tree, const ghostos_volume_tree_node *leaf,
    const ghostos_volume_record *record, inserted *result) {
    ghostos_volume_record records[GHOSTOS_VOLUME_TREE_KEYS + 1] = {0};
    ghostos_volume_tree_node node = { .kind = 1 };
    ghostos_volume_tree_key key = key_of(record);
    size_t position = 0, i, count = (size_t)leaf->length + 1;
    int status;
    while (position < leaf->length) {
        ghostos_volume_tree_key existing = key_of(&leaf->entries.records[position]);
        int order = compare(&existing, &key);
        if (!order) return 4;
        if (order >= 0) break;
        position += 1;
    }
    for (i = 0; i < position; ++i) records[i] = leaf->entries.records[i];
    records[position] = *record;
    for (i = position; i < leaf->length; ++i) records[i + 1] = leaf->entries.records[i];
    node.length = (uint8_t)(count <= GHOSTOS_VOLUME_TREE_KEYS ? count : count / 2);
    for (i = 0; i < node.length; ++i) node.entries.records[i] = records[i];
    status = allocate(tree, &node, &result->left);
    if (status) return status;
    result->split = count > GHOSTOS_VOLUME_TREE_KEYS;
    if (result->split) {
        size_t middle = count / 2;
        node = (ghostos_volume_tree_node){ .kind = 1, .length = (uint8_t)(count - middle) };
        for (i = 0; i < node.length; ++i) node.entries.records[i] = records[middle + i];
        status = allocate(tree, &node, &result->right);
        if (status) return status;
        result->separator = key_of(&records[middle]);
    }
    return 0;
}
static int insert_branch(ghostos_volume_tree *tree, const ghostos_volume_tree_frame *frame, inserted *result) {
    ghostos_volume_tree_key keys[GHOSTOS_VOLUME_TREE_KEYS + 1] = {0};
    uint32_t children[GHOSTOS_VOLUME_TREE_KEYS + 2] = {0};
    ghostos_volume_tree_node node = { .kind = 2 };
    size_t i, child = frame->child, length = frame->node.length;
    size_t count = length + (result->split ? 1u : 0u);
    int status;
    for (i = 0; i < child; ++i) {
        keys[i] = frame->node.entries.branch.keys[i];
        children[i] = frame->node.entries.branch.children[i];
    }
    children[child] = result->left;
    if (result->split) {
        keys[child] = result->separator;
        children[child + 1] = result->right;
    }
    for (i = child; i < length; ++i) keys[i + (result->split ? 1u : 0u)] = frame->node.entries.branch.keys[i];
    for (i = child + 1; i <= length; ++i) children[i + (result->split ? 1u : 0u)] = frame->node.entries.branch.children[i];
    node.length = (uint8_t)(count <= GHOSTOS_VOLUME_TREE_KEYS ? count : count / 2);
    for (i = 0; i < node.length; ++i) node.entries.branch.keys[i] = keys[i];
    for (i = 0; i <= node.length; ++i) node.entries.branch.children[i] = children[i];
    status = allocate(tree, &node, &result->left);
    if (status) return status;
    result->split = count > GHOSTOS_VOLUME_TREE_KEYS;
    if (result->split) {
        size_t middle = count / 2;
        result->separator = keys[middle];
        node = (ghostos_volume_tree_node){ .kind = 2, .length = (uint8_t)(count - middle - 1) };
        for (i = 0; i < node.length; ++i) node.entries.branch.keys[i] = keys[middle + 1 + i];
        for (i = 0; i <= node.length; ++i) node.entries.branch.children[i] = children[middle + 1 + i];
        status = allocate(tree, &node, &result->right);
        if (status) return status;
    }
    return 0;
}
int ghostos_volume_tree_insert(ghostos_volume_tree *tree, uint32_t root,
    const ghostos_volume_record *record, uint32_t *new_root) {
    ghostos_volume_tree_node node = { .kind = 1 };
    ghostos_volume_tree_key key;
    inserted result = {0};
    size_t depth = 0;
    uint32_t id = root;
    int status = valid_tree(tree, record);
    if (status) return status;
    key = key_of(record);
    while (id) {
        size_t child = 0;
        status = read_node(tree, id, &node);
        if (status) return status;
        if (node.kind == 1) break;
        if (depth >= tree->block_count) return 1;
        if (depth == tree->frame_capacity) return 2;
        while (child < node.length && compare(&key, &node.entries.branch.keys[child]) >= 0) child += 1;
        tree->frames[depth++] = (ghostos_volume_tree_frame){ .node = node, .child = child };
        id = node.entries.branch.children[child];
        if (!id) return 1;
    }
    status = insert_leaf(tree, &node, record, &result);
    if (status) return status;
    while (depth) {
        status = insert_branch(tree, &tree->frames[--depth], &result);
        if (status) return status;
    }
    if (result.split) {
        node = (ghostos_volume_tree_node){ .kind = 2, .length = 1 };
        node.entries.branch.keys[0] = result.separator;
        node.entries.branch.children[0] = result.left;
        node.entries.branch.children[1] = result.right;
        status = allocate(tree, &node, &result.left);
        if (status) return status;
    }
    *new_root = result.left;
    return 0;
}
int ghostos_volume_tree_replace(ghostos_volume_tree *tree, uint32_t root,
    const ghostos_volume_record *record, uint32_t *new_root) {
    ghostos_volume_tree_node node;
    ghostos_volume_tree_key key;
    size_t depth = 0, i;
    uint32_t id = root, updated;
    int status = valid_tree(tree, record);
    if (status) return status;
    key = key_of(record);
    if (!id) return 1;
    for (;;) {
        status = read_node(tree, id, &node);
        if (status) return status;
        if (node.kind == 2) {
            if (depth >= tree->block_count) return 1;
            if (depth == tree->frame_capacity) return 2;
            tree->frames[depth++] = (ghostos_volume_tree_frame){ .node = node, .child = 0 };
            id = node.entries.branch.children[0];
            continue;
        }
        for (i = 0; i < node.length; ++i) {
            ghostos_volume_tree_key existing = key_of(&node.entries.records[i]);
            if (same_key(&existing, &key)) break;
        }
        if (i < node.length) {
            node.entries.records[i] = *record;
            status = allocate(tree, &node, &updated);
            if (status) return status;
            while (depth) {
                ghostos_volume_tree_frame *frame = &tree->frames[--depth];
                frame->node.entries.branch.children[frame->child] = updated;
                status = allocate(tree, &frame->node, &updated);
                if (status) return status;
            }
            *new_root = updated;
            return 0;
        }
        while (depth && tree->frames[depth - 1].child == tree->frames[depth - 1].node.length) depth -= 1;
        if (!depth) return 5;
        tree->frames[depth - 1].child += 1;
        id = tree->frames[depth - 1].node.entries.branch.children[tree->frames[depth - 1].child];
    }
}
