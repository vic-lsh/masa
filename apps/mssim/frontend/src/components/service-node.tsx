import { Handle, Position } from "reactflow";
import { memo } from "react";

interface ServiceNodeData {
  serviceName: string;
  methodName: string;
  port?: number;
  error_rate: {
    type: string;
    parameters: Record<string, number>;
  };
  latency_distribution: {
    type: string;
    parameters: Record<string, number>;
  };
  requests_per_second?: number;
}

interface ServiceNodeProps {
  data: ServiceNodeData;
  selected?: boolean;
}

const ServiceNode = memo(({ data, selected }: ServiceNodeProps) => {
  const getNodeColor = () => {
    if (data.requests_per_second) return "#FEC700"; // Entry point
    if (data.error_rate.parameters.p > 0.1) return "#84754D"; // High error rate
    if (
      data.latency_distribution.parameters.mean > 200 ||
      data.latency_distribution.parameters.value > 200
    )
      return "#B7A57A"; // High latency
    return "#4B2E82"; // Default
  };

  return (
    <div
      className="px-4 py-3 shadow-lg rounded-lg border-2 bg-white min-w-[200px]"
      style={{
        borderColor: selected ? "#FEC700" : "#32006D",
        backgroundColor: "white",
      }}
    >
      <Handle
        type="target"
        position={Position.Top}
        className="w-3 h-3"
        style={{ backgroundColor: "#32006D" }}
      />

      <div className="text-center">
        <div
          className="text-sm font-semibold mb-1 px-2 py-1 rounded"
          style={{ backgroundColor: getNodeColor(), color: "white" }}
        >
          {data.serviceName}
        </div>
        <div className="text-xs font-medium mb-2" style={{ color: "#32006D" }}>
          {data.methodName}
        </div>

        {data.port && (
          <div className="text-xs mb-1" style={{ color: "#84754D" }}>
            Port: {data.port}
          </div>
        )}

        <div className="text-xs space-y-1">
          <div style={{ color: "#32006D" }}>
            Error: {(data.error_rate.parameters.p * 100).toFixed(1)}%
          </div>
          <div style={{ color: "#32006D" }}>
            Latency:{" "}
            {data.latency_distribution.parameters.mean ||
              data.latency_distribution.parameters.value}
            ms
          </div>
          {data.requests_per_second && (
            <div className="font-semibold" style={{ color: "#FEC700" }}>
              Entry: {data.requests_per_second} RPS
            </div>
          )}
        </div>
      </div>

      <Handle
        type="source"
        position={Position.Bottom}
        className="w-3 h-3"
        style={{ backgroundColor: "#32006D" }}
      />
    </div>
  );
});

ServiceNode.displayName = "ServiceNode";

export default ServiceNode;
