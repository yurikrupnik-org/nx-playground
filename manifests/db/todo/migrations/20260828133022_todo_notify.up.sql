-- Realtime UI support: make the DATABASE the source of todo change events.
--
-- Before this, browsers only saw changes that happened inside the todo-api
-- process that owned their SSE/WebSocket connection: a second API replica, the
-- todo-worker, the todo CLI or a plain `psql` UPDATE moved the data without any
-- UI noticing. A row-level AFTER trigger emits NOTIFY on `todo_events` instead,
-- so every committed change reaches every connected client regardless of which
-- writer made it.
--
-- Payload is deliberately tiny — `{"kind": ..., "id": ...}` — because NOTIFY
-- payloads are capped (8000 bytes) and `todos.description` is unbounded TEXT.
-- The listener hydrates the full row itself, which also refreshes its cache.
--
-- NOTIFY is transactional: it fires on COMMIT, so clients never observe a change
-- that later rolls back.

CREATE OR REPLACE FUNCTION todo_notify() RETURNS trigger AS $$
DECLARE
    event_kind text;
    row_id     uuid;
BEGIN
    IF TG_OP = 'INSERT' THEN
        event_kind := 'created';
        row_id     := NEW.id;
    ELSIF TG_OP = 'DELETE' THEN
        event_kind := 'deleted';
        row_id     := OLD.id;
    ELSE
        row_id := NEW.id;
        -- Mirror the TodoEventKind variants the service publishes, so a change
        -- made in SQL is indistinguishable from one made through the API.
        IF NEW.completed AND NOT OLD.completed THEN
            event_kind := 'completed';
        ELSIF OLD.completed AND NOT NEW.completed THEN
            event_kind := 'uncompleted';
        ELSE
            event_kind := 'updated';
        END IF;
    END IF;

    PERFORM pg_notify(
        'todo_events',
        json_build_object('kind', event_kind, 'id', row_id)::text
    );

    -- AFTER ... FOR EACH ROW: the return value is ignored.
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS todos_notify ON todos;

CREATE TRIGGER todos_notify
    AFTER INSERT OR UPDATE OR DELETE ON todos
    FOR EACH ROW EXECUTE FUNCTION todo_notify();
