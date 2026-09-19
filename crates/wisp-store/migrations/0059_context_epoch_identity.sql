-- Undo removes epoch rows but keeps CompactionUndone events. Never reuse their
-- identities, including after rewind, restart, or replacing model messages.
UPDATE frames SET context_epoch_high_water = MAX(
    context_epoch_high_water, head_epoch,
    COALESCE((SELECT MAX(epoch) FROM context_epochs WHERE frame_id=frames.id), 0),
    COALESCE((SELECT MAX(CAST(json_extract(event_json,'$.epoch') AS INTEGER))
              FROM session_ui_events WHERE frame_id=frames.id
              AND json_extract(event_json,'$.kind') IN ('Compaction','CompactionUndone')), 0)
);

-- Older automatic flags were linked by ui_event_seq but their JSON still had
-- epoch:null. Preserve the link on reload as well as on live refresh.
UPDATE session_ui_events SET event_json = json_set(event_json, '$.epoch',
    (SELECT epoch FROM context_epochs WHERE frame_id=session_ui_events.frame_id
       AND ui_event_seq=session_ui_events.seq))
WHERE json_extract(event_json,'$.kind')='Compaction'
  AND EXISTS (SELECT 1 FROM context_epochs WHERE frame_id=session_ui_events.frame_id
                AND ui_event_seq=session_ui_events.seq);
