use std::collections::HashMap;

use drillx::Solution;
use solana_sdk::pubkey::Pubkey;

use crate::SubmittedSolution;

pub struct Rewards {}

// TODO: optimize this
pub fn calculate_rewards(
    epoch_solutions: &[SubmittedSolution],
    reward: u32,
) -> HashMap<Pubkey, u32> {
    // 1. ensure that there's only one solution for each miner
    // 2. pick the highest solution for each miner
    // 3. there will be an interval of difficulty [MIN, MAX]
    // weight is defined as w_i = 2^(difficulty - MIN)/(sum_weights)

    let unique_solutions =
        epoch_solutions
            .iter()
            .fold(HashMap::<Pubkey, SubmittedSolution>::new(), |mut acc, x| {
                let e = acc.entry(x.miner_authority).or_insert(x.clone());
                let proposed_solution = x.solution;
                let proposed_difficulty = get_difficulty(proposed_solution);
                let best_solution = e.solution;
                let best_difficulty = get_difficulty(best_solution);
                if proposed_difficulty > best_difficulty {
                    *e = x.clone();
                }
                acc
            });

    let min_reward = unique_solutions
        .iter()
        .fold(100, |mut acc, (who, solution)| {
            let diff = get_difficulty(solution.solution);
            if diff < acc {
                acc = diff;
            }
            acc
        });

    let sum_weights = unique_solutions.iter().fold(0, |mut acc, (who, solution)| {
        let diff = get_difficulty(solution.solution);
        acc += 2u32.pow(diff - min_reward);
        acc
    });

    let rewards = unique_solutions
        .iter()
        .fold(HashMap::new(), |mut acc, (_, solution)| {
            let diff = get_difficulty(solution.solution);
            acc.insert(
                solution.miner_authority,
                (2u32.pow(diff - min_reward) / sum_weights) * reward,
            );
            acc
        });
    rewards
}

fn get_difficulty(solution: [u8; 24]) -> u32 {
    let s = Solution::new(
        solution[0..16].try_into().unwrap(),
        solution[16..24].try_into().unwrap(),
    );
    let hash = s.to_hash();
    let difficulty = hash.difficulty();
    difficulty
}
