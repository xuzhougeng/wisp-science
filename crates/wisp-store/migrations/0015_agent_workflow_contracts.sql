ALTER TABLE agent_workflow_steps
    ADD COLUMN input_contract_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE agent_workflow_steps
    ADD COLUMN output_contract_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE agent_workflow_steps
    ADD COLUMN budget_json TEXT NOT NULL DEFAULT '{}';
