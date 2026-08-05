# 14. Compute, LLMs, Agents, and Semantic Memory

AI is treated as a system workload, not a privileged exception.

## `synos-compute`

The compute runtime supports Candle and Burn through a small native contract without C, C++, CUDA, or POSIX dependencies. Tensor metadata points into capability-mapped IPC regions.

A tensor view validates:

- rank and shape;
- strides;
- total size and integer overflow;
- alignment;
- read/write access;
- shared-buffer bounds.

GPU and NPU work uses isolated Ring 3 drivers, bounded descriptors, capability-controlled BAR/interrupt/doorbell access, and generation-checked asynchronous queues.

## `synos-llm`

The LLM runtime presents local RAM, CXL, and remote memory leases as one contiguous virtual model allocation. Model placement is hidden behind the memory contract, but the failure and quota rules remain explicit.

KV caches grow in stable token-addressed segments. Allocation policy prefers local RAM, then CXL, then Layer-2 memory, and mirrors every segment. Existing token addresses do not move when context grows.

## Inference recovery

Each inference request has a checksummed recovery record on two journal nodes. A token checkpoint is committed only after both copies acknowledge it.

```text
generate tokens -> checkpoint -> journal A + journal B -> commit
                                      |
                           one node fails -> resume last commit
```

The request identity stays stable while mirrored model and KV pages resolve through the fabric.

## Semantic memory

`synos-agentd` provides a capability-scoped semantic memory and context bus. It handles indexing, vector encoding, similarity search, freshness/decay, authorization filtering, zero-copy retrieval, and garbage collection.

An agent should not see every memory object. Retrieval is filtered by authority before records are returned.

## Agent execution

`synos-agent-bridge` connects agents to typed tools. It exports live schemas, derives exact task capabilities, applies replay protection, and runs scripts in private filesystem sandboxes.

Long-running agents use immutable SynFS execution snapshots. The newest snapshot pins a complete CoW generation so a crashed process can restore its stack and state.

## Easy example: an AI task

```text
agent asks for: read /DATA/report, summarize, write /DATA/summary
system grants: READ report + WRITE summary, expires in 5 minutes
agent runs in: private SynFS stage
system checks: tool schema, input bounds, status, audit event
approve: publish summary generation
reject: discard stage, keep original data
```

The model is not the authority. The capability and sandbox are.

