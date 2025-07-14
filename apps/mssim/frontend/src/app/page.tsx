/* eslint-disable @typescript-eslint/no-unused-vars */
/* eslint-disable @typescript-eslint/no-explicit-any */
"use client";

import type React from "react";

import { useState, useCallback, useRef } from "react";
import ReactFlow, {
  type Node,
  type Edge,
  addEdge,
  useNodesState,
  useEdgesState,
  Controls,
  MiniMap,
  Background,
  BackgroundVariant,
  type Connection,
  type NodeTypes,
} from "reactflow";
import "reactflow/dist/style.css";
import { Download, Plus } from "lucide-react";
import { saveAs } from "file-saver";
import ServiceNode from "@/components/service-node";
import NodeEditor from "@/components/node-editor";
import FileUpload from "@/components/file-upload";

const nodeTypes: NodeTypes = {
  service: ServiceNode,
};

interface ServiceData {
  id: string;
  serviceName: string;
  methodName: string;
  port?: number;
  calls: string[];
  latency_distribution: {
    type: string;
    parameters: Record<string, number>;
  };
  error_rate: {
    type: string;
    parameters: Record<string, number>;
  };
  requests_per_second?: number;
}

export default function GraphEditor() {
  const [nodes, setNodes, onNodesChange] = useNodesState([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState([]);
  const [selectedNode, setSelectedNode] = useState<Node<ServiceData> | null>(
    null
  );
  const [jsonData, setJsonData] = useState<any>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const onConnect = useCallback(
    (params: Edge | Connection) => setEdges((eds) => addEdge(params, eds)),
    [setEdges]
  );

  const parseJsonToGraph = (data: any) => {
    const newNodes: Node<ServiceData>[] = [];
    const newEdges: Edge[] = [];
    let nodeId = 0;
    const nodeMap = new Map<string, string>();

    // Create nodes for each service method
    Object.entries(data.services).forEach(
      ([serviceName, service]: [string, any]) => {
        Object.entries(service.methods).forEach(
          ([methodName, method]: [string, any]) => {
            const id = `node-${nodeId++}`;
            const fullName = `${serviceName}.${methodName}`;
            nodeMap.set(fullName, id);

            // Find if this is an entry point
            const entryPoint = data.load?.entry_points?.find(
              (ep: any) =>
                ep.service === serviceName && ep.method === methodName
            );

            newNodes.push({
              id,
              type: "service",
              position: { x: Math.random() * 800, y: Math.random() * 600 },
              data: {
                id,
                serviceName,
                methodName,
                port: service.port,
                calls: method.calls?.flat() || [],
                latency_distribution: method.latency_distribution,
                error_rate: method.error_rate,
                requests_per_second: entryPoint?.requests_per_second,
              },
            });
          }
        );
      }
    );

    // Create edges based on calls
    newNodes.forEach((node) => {
      node.data.calls.forEach((call: string) => {
        const targetId = nodeMap.get(call);
        if (targetId) {
          newEdges.push({
            id: `edge-${node.id}-${targetId}`,
            source: node.id,
            target: targetId,
            type: "smoothstep",
          });
        }
      });
    });

    setNodes(newNodes);
    setEdges(newEdges);
    setJsonData(data);
  };

  const exportToJson = () => {
    const services: any = {};
    const entryPoints: any[] = [];

    // Group nodes by service
    const serviceGroups = new Map<string, Node<ServiceData>[]>();
    nodes.forEach((node) => {
      const serviceName = node.data.serviceName;
      if (!serviceGroups.has(serviceName)) {
        serviceGroups.set(serviceName, []);
      }
      serviceGroups.get(serviceName)!.push(node);
    });

    // Build services object
    serviceGroups.forEach((serviceNodes, serviceName) => {
      const methods: any = {};
      let port: number | undefined;

      serviceNodes.forEach((node) => {
        const methodName = node.data.methodName;
        port = node.data.port || port;

        // Find calls for this node
        const calls = edges
          .filter((edge) => edge.source === node.id)
          .map((edge) => {
            const targetNode = nodes.find((n) => n.id === edge.target);
            return targetNode
              ? `${targetNode.data.serviceName}.${targetNode.data.methodName}`
              : "";
          })
          .filter(Boolean);

        methods[methodName] = {
          calls: calls.length > 0 ? [calls] : [],
          latency_distribution: node.data.latency_distribution,
          error_rate: node.data.error_rate,
        };

        // Check if this is an entry point
        if (node.data.requests_per_second) {
          entryPoints.push({
            service: serviceName,
            method: methodName,
            requests_per_second: node.data.requests_per_second,
          });
        }
      });

      services[serviceName] = {
        port: port || 50051,
        methods,
      };
    });

    const exportData = {
      services,
      load: {
        entry_points: entryPoints,
      },
    };
    console.log(exportData);
    const blob = new Blob([JSON.stringify(exportData, null, 2)], {
      type: "application/json",
    });
    saveAs(blob, "microservice-graph.json");
  };

  const addNewNode = () => {
    const newNode: Node<ServiceData> = {
      id: `node-${Date.now()}`,
      type: "service",
      position: { x: Math.random() * 400 + 100, y: Math.random() * 400 + 100 },
      data: {
        id: `node-${Date.now()}`,
        serviceName: "new-service",
        methodName: "new-method",
        port: 50051,
        calls: [],
        latency_distribution: {
          type: "constant",
          parameters: { value: 100.0 },
        },
        error_rate: {
          type: "bernoulli",
          parameters: { p: 0.0 },
        },
      },
    };
    setNodes((nds) => [...nds, newNode]);
  };

  const onNodeClick = useCallback(
    (event: React.MouseEvent, node: Node<ServiceData>) => {
      setSelectedNode(node);
    },
    []
  );

  const updateNodeData = (nodeId: string, newData: Partial<ServiceData>) => {
    setNodes((nds) =>
      nds.map((node) =>
        node.id === nodeId
          ? { ...node, data: { ...node.data, ...newData } }
          : node
      )
    );
    if (selectedNode && selectedNode.id === nodeId) {
      setSelectedNode({
        ...selectedNode,
        data: { ...selectedNode.data, ...newData },
      });
    }
  };

  return (
    <div
      className="h-screen flex flex-col"
      style={{ backgroundColor: "white" }}
    >
      {/* Header */}
      <div
        className="h-16 border-b-2 flex items-center justify-between px-6"
        style={{ borderColor: "#32006D", backgroundColor: "#32006D" }}
      >
        <h1 className="text-2xl font-bold" style={{ color: "white" }}>
          Microservice Graph Editor
        </h1>
        <div className="flex gap-4">
          <FileUpload onFileLoad={parseJsonToGraph} />
          <button
            onClick={addNewNode}
            className="flex items-center gap-2 px-4 py-2 rounded-lg text-white hover:bg-white hover:text-[#32006D] transition-colors"
            style={{ backgroundColor: "#4B2E82" }}
          >
            <Plus size={20} />
            Add Node
          </button>
          <button
            onClick={exportToJson}
            className="flex items-center gap-2 px-4 py-2 rounded-lg text-white hover:bg-white hover:text-[#32006D] transition-colors"
            style={{ backgroundColor: "#FEC700", color: "#32006D" }}
          >
            <Download size={20} />
            Export JSON
          </button>
        </div>
      </div>

      <div className="flex-1 flex">
        {/* Graph Area */}
        <div className="flex-1 relative">
          <ReactFlow
            nodes={nodes}
            edges={edges}
            onNodesChange={onNodesChange}
            onEdgesChange={onEdgesChange}
            onConnect={onConnect}
            onNodeClick={onNodeClick}
            nodeTypes={nodeTypes}
            fitView
            className="bg-white"
          >
            <Controls />
            <MiniMap />
            <Background
              variant={BackgroundVariant.Dots}
              gap={12}
              size={1}
              color="#32006D"
            />
          </ReactFlow>
        </div>

        {/* Side Panel */}
        {selectedNode && (
          <div
            className="w-80 border-l-2 p-4 overflow-y-auto"
            style={{ borderColor: "#32006D" }}
          >
            <NodeEditor
              node={selectedNode}
              onUpdate={(data) => updateNodeData(selectedNode.id, data)}
              onClose={() => setSelectedNode(null)}
            />
          </div>
        )}
      </div>
    </div>
  );
}
