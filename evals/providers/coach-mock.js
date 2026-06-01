// Offline mock Coach provider for promptfoo.
// Returns a canned, shame-free, single-action response so the eval HARNESS and
// the deterministic asserts (shame denylist + length) can run with no network /
// no LLM. Swap to a real provider in promptfooconfig.yaml for model-graded
// (llm-rubric) evals. This proves the eval wiring works; it does NOT judge
// real model quality.
//
// promptfoo custom-provider contract: export an object with id + callApi().
module.exports = {
  id: () => 'coach-mock',
  async callApi(_prompt, _context) {
    // A deliberately shame-free, single concrete next action.
    const output =
      '明天的一個小行動：把今天那 50 分鐘的專注時段，固定排在早上第一件事 —— ' +
      '先做 25 分鐘就好，做到了就算數。你已經有節奏了，這只是把它釘在固定時間。';
    return { output };
  },
};
