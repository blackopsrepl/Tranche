//! Rebuild assignment inputs and validate both bindings and hard feasibility.
use super::{
    AssignmentPlan, AssignmentTask,
    preprocessing::{NOT_REQUIRED, QUALIFIED, Taxonomy, current, probabilities, records},
    team,
};
use crate::{
    domain::{batch::park_state, judge::Judgment, pr::Prs},
    report::{REPOSITORY, Root},
    util::digest,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};

pub struct Prepared {
    pub plan: AssignmentPlan,
    pub input_binding: String,
}
pub fn prepare(
    root: &Root,
    prs: &Prs,
    judgments: &HashMap<u64, Judgment>,
    dupes: &Value,
    batches: Option<&Value>,
    report_binding: &Value,
) -> Result<Prepared, String> {
    let taxonomy = Taxonomy::load(root)?;
    let members = team::members(root, &taxonomy)?;
    if members.is_empty() {
        return Err("no synthetic team resumes; add input/team/*.md".into());
    }
    let requirements = records(&root.out_dir().join("requirements.jsonl"))?;
    let parks = park_state(dupes, judgments, prs);
    let mut groups: Vec<Vec<u64>> = dupes["confirmed_groups"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|g| {
            g.as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_u64)
                .collect()
        })
        .collect();
    let mut grouped: HashSet<u64> = groups.iter().flatten().copied().collect();
    let mut nums: Vec<u64> = prs.iter().map(|p| p.number).collect();
    nums.sort();
    for n in nums {
        if grouped.insert(n) {
            groups.push(vec![n]);
        }
    }
    let mut tasks = Vec::new();
    for mut numbers in groups {
        numbers.sort();
        let first = *numbers.first().ok_or("empty duplicate group")?;
        let mut skills = std::collections::BTreeSet::new();
        let mut known = true;
        let mut security = false;
        for &n in &numbers {
            let pr = prs.get(n).ok_or("duplicate group references missing PR")?;
            let req = current(
                &requirements,
                &n.to_string(),
                &super::preprocessing::requirement_state(pr),
                &taxonomy,
                false,
            );
            if let Some(r) = req {
                for (id, p) in probabilities(&r.answers, &taxonomy) {
                    match p {
                        Some(p) if p >= QUALIFIED => {
                            skills.insert(id);
                        }
                        Some(p) if p <= NOT_REQUIRED => {}
                        _ => known = false,
                    }
                }
            } else {
                known = false;
            }
            match judgments
                .get(&n)
                .and_then(|j| j.metric("security_flag", "noul"))
            {
                Some(p) => security |= p >= 0.5,
                None => known = false,
            }
            known &= !parks.contains_key(&n);
        }
        let category = judgments
            .get(&first)
            .map(|j| j.category())
            .unwrap_or_default();
        let mut t = AssignmentTask::new(
            first,
            String::new(),
            skills.into_iter().collect(),
            security,
            (numbers.len() > 1).then(|| format!("group-{first}")),
            category,
        );
        t.numbers = numbers;
        t.evidence_known = known;
        tasks.push(t);
    }
    tasks.sort_by_key(|t| t.pr_number);
    let plan = AssignmentPlan::new(members, tasks);
    let mut source = BTreeMap::new();
    for resume in team::resumes(root)? {
        source.insert(resume.id.clone(), digest(&resume.state()));
    }
    let qualification_cache = records(&root.out_dir().join("qualifications.jsonl"))?;
    let input_binding = digest(
        &json!({"version":1, "plan":plan, "report_binding":report_binding,
        "dupes":digest(dupes), "batches":batches.map(digest), "resumes":source,
        "qualifications":qualification_cache, "requirements":requirements,
        "questions":super::preprocessing::questions(&taxonomy, false),
        "solver_config":include_str!("solver.toml"), "policy":"atomic-units;hard-pr-cap;skills;security;unknown-hold;balance;affinity-v1"}),
    );
    Ok(Prepared {
        plan,
        input_binding,
    })
}

pub fn payload(prepared: &Prepared, plan: &AssignmentPlan) -> Result<Value, String> {
    let mut load = vec![0usize; plan.members.len()];
    let mut rows = Vec::new();
    for t in &plan.tasks {
        let member = match t.member_idx {
            Some(idx) => {
                let m = plan.members.get(idx).ok_or("invalid assignee index")?;
                if !t.evidence_known
                    || !m.evidence_known
                    || !m.covers(&t.required_skills)
                    || (t.security_flag
                        && !m.qualified_skills.iter().any(|s| s == "security-review"))
                {
                    return Err("assignment violates skill, unknown or security gate".into());
                }
                load[idx] += t.numbers.len();
                if load[idx] > m.capacity {
                    return Err("assignment exceeds hard PR capacity".into());
                }
                Some(m.id.clone())
            }
            None => None,
        };
        for n in &t.numbers {
            rows.push(json!({"number":n, "member_id":member, "unit":t.id, "required_skills":t.required_skills,
                "security_priority":t.security_flag, "evidence_known":t.evidence_known,
                "reason":if member.is_some() { "proposed" } else if !t.evidence_known { "unknown_or_parked" } else { "unassigned_capacity_or_coverage" }}));
        }
    }
    rows.sort_by_key(|r| r["number"].as_u64());
    let members: Vec<_> = plan.members.iter().enumerate().map(|(i,m)| json!({"id":m.id,
        "capacity":m.capacity, "load":load[i], "qualified_skills":m.qualified_skills, "evidence_known":m.evidence_known})).collect();
    let mut result = json!({"format_version":1, "repo":REPOSITORY, "input_binding":prepared.input_binding,
        "source":"synthetic-resume-simulation", "meaning":"AI-assisted proposed owners only. Synthetic people, not real GitHub handles. Never an approval, reservation or automatic mention.",
        "members":members, "assignments":rows});
    result["output_digest"] = json!(digest(&result));
    Ok(result)
}
pub fn validate(
    root: &Root,
    assignments: &Value,
    prs: &Prs,
    judgments: &HashMap<u64, Judgment>,
    dupes: &Value,
    batches: Option<&Value>,
    report_binding: &Value,
) -> Result<(), String> {
    let prepared = prepare(root, prs, judgments, dupes, batches, report_binding)?;
    let mut plan = prepared.plan.clone();
    let rows = assignments["assignments"]
        .as_array()
        .ok_or("assignment rows missing")?;
    let mut owners = BTreeMap::new();
    for row in rows {
        let number = row["number"].as_u64().ok_or("invalid assignment number")?;
        let owner = match &row["member_id"] {
            Value::Null => None,
            Value::String(id) => Some(
                plan.members
                    .iter()
                    .position(|m| &m.id == id)
                    .ok_or("unknown assignee")?,
            ),
            _ => return Err("invalid assignee".into()),
        };
        if owners.insert(number, owner).is_some() {
            return Err("duplicate assignment row".into());
        }
    }
    if owners.len() != prs.len() {
        return Err("assignment coverage mismatch".into());
    }
    for task in &mut plan.tasks {
        let owner = *owners
            .get(&task.pr_number)
            .ok_or("assignment row missing")?;
        if task.numbers.iter().any(|n| owners.get(n) != Some(&owner)) {
            return Err("atomic duplicate group split".into());
        }
        task.member_idx = owner;
    }
    if payload(&prepared, &plan)? != *assignments {
        return Err("assignments.json stale, foreign or modified; rerun assign".into());
    }
    Ok(())
}
pub fn assigned(assignments: Option<&Value>, number: u64) -> Option<&Value> {
    assignments?
        .get("assignments")?
        .as_array()?
        .iter()
        .find(|r| r["number"].as_u64() == Some(number) && r["member_id"].is_string())
}
