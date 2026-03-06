import sys
sys.setrecursionlimit(1100000)

class TreeNode:
    __slots__ = ('value', 'left', 'right')
    def __init__(self, value, left, right):
        self.value = value
        self.left = left
        self.right = right

def build_tree(depth):
    if depth == 0:
        return TreeNode(0, None, None)
    left = build_tree(depth - 1)
    right = build_tree(depth - 1)
    return TreeNode(left.value + right.value + 1, left, right)

def gc_pressure_tree(depth, iterations):
    total = 0
    for i in range(iterations):
        tree = build_tree(depth)
        total += tree.value
    return total
print(gc_pressure_tree(20, 5))
