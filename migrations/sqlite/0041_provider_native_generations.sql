-- Monotonic native-home reservations; cancellation never releases a number.
CREATE TABLE provider_native_generations (
    owner_user_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    machine_id TEXT NOT NULL,
    generation BIGINT NOT NULL CHECK (generation > 0),
    PRIMARY KEY (owner_user_id, account_id, machine_id)
);

-- Preserve any native completion evidence recorded before counters existed.
INSERT INTO provider_native_generations
    (owner_user_id, account_id, machine_id, generation)
SELECT owner_user_id,
       json_extract(value, '$.input.account_id'),
       json_extract(value, '$.input.machine_id'),
       MAX(json_extract(value, '$.native_completion.profile.generation'))
FROM provider_login_operations
WHERE json_extract(value, '$.native_completion.profile.generation') IS NOT NULL
GROUP BY owner_user_id, json_extract(value, '$.input.account_id'),
         json_extract(value, '$.input.machine_id');
