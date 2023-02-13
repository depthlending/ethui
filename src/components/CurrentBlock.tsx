import { useState } from "react";
import { requestToBackground } from "../messenger";

export function CurrentBlock() {
  const [block, setBlock] = useState(0);

  async function getBlock() {
    console.log("getBlock");
    const resp = await requestToBackground({
      type: "eth",
      message: { method: "eth_getBlockNumber", params: [] },
    });
    console.log(resp);
    setBlock(block);
  }

  return (
    <div>
      <button onClick={() => getBlock()}>Get block</button>
      <p>Block: {block}</p>
    </div>
  );
}