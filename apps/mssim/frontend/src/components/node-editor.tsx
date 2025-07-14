/* eslint-disable @typescript-eslint/no-explicit-any */
"use client";

import type React from "react";

import { X, Save } from "lucide-react";
import { useState, useEffect } from "react";

interface NodeEditorProps {
  node: any;
  onUpdate: (data: any) => void;
  onClose: () => void;
}

export default function NodeEditor({
  node,
  onUpdate,
  onClose,
}: NodeEditorProps) {
  const [formData, setFormData] = useState(node.data);

  useEffect(() => {
    setFormData(node.data);
  }, [node]);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    onUpdate(formData);
  };

  const handleInputChange = (field: string, value: any) => {
    setFormData((prev: any) => ({
      ...prev,
      [field]: value,
    }));
  };

  const handleNestedChange = (parent: string, child: string, value: any) => {
    setFormData((prev: any) => ({
      ...prev,
      [parent]: {
        ...prev[parent],
        [child]: value,
      },
    }));
  };

  const handleParameterChange = (parent: string, param: string, value: any) => {
    setFormData((prev: any) => ({
      ...prev,
      [parent]: {
        ...prev[parent],
        parameters: {
          ...prev[parent].parameters,
          [param]: value,
        },
      },
    }));
  };

  return (
    <div className="h-full flex flex-col">
      <div
        className="flex items-center justify-between mb-4 pb-2 border-b-2"
        style={{ borderColor: "#32006D" }}
      >
        <h3 className="text-lg font-semibold" style={{ color: "#32006D" }}>
          Edit Node
        </h3>
        <button
          onClick={onClose}
          className="p-1 rounded hover:bg-gray-100"
          style={{ color: "#32006D" }}
        >
          <X size={20} />
        </button>
      </div>

      <form onSubmit={handleSubmit} className="flex-1 space-y-4">
        <div>
          <label
            className="block text-sm font-medium mb-1"
            style={{ color: "#32006D" }}
          >
            Service Name
          </label>
          <input
            type="text"
            value={formData.serviceName}
            onChange={(e) => handleInputChange("serviceName", e.target.value)}
            className="w-full px-3 py-2 border rounded-lg focus:outline-none focus:ring-2"
            style={{ borderColor: "#32006D" }}
          />
        </div>

        <div>
          <label
            className="block text-sm font-medium mb-1"
            style={{ color: "#32006D" }}
          >
            Method Name
          </label>
          <input
            type="text"
            value={formData.methodName}
            onChange={(e) => handleInputChange("methodName", e.target.value)}
            className="w-full px-3 py-2 border rounded-lg focus:outline-none focus:ring-2"
            style={{ borderColor: "#32006D" }}
          />
        </div>

        <div>
          <label
            className="block text-sm font-medium mb-1"
            style={{ color: "#32006D" }}
          >
            Port
          </label>
          <input
            type="number"
            value={formData.port || ""}
            onChange={(e) =>
              handleInputChange(
                "port",
                Number.parseInt(e.target.value) || undefined
              )
            }
            className="w-full px-3 py-2 border rounded-lg focus:outline-none focus:ring-2"
            style={{ borderColor: "#32006D" }}
          />
        </div>

        <div>
          <label
            className="block text-sm font-medium mb-1"
            style={{ color: "#32006D" }}
          >
            Requests Per Second (Entry Point)
          </label>
          <input
            type="number"
            value={formData.requests_per_second || ""}
            onChange={(e) =>
              handleInputChange(
                "requests_per_second",
                Number.parseFloat(e.target.value) || undefined
              )
            }
            className="w-full px-3 py-2 border rounded-lg focus:outline-none focus:ring-2"
            style={{ borderColor: "#32006D" }}
            placeholder="Leave empty if not entry point"
          />
        </div>

        <div className="space-y-2">
          <h4 className="font-medium" style={{ color: "#32006D" }}>
            Error Rate
          </h4>
          <div>
            <label className="block text-xs mb-1" style={{ color: "#84754D" }}>
              Type
            </label>
            <select
              value={formData.error_rate.type}
              onChange={(e) =>
                handleNestedChange("error_rate", "type", e.target.value)
              }
              className="w-full px-3 py-2 border rounded-lg focus:outline-none focus:ring-2"
              style={{ borderColor: "#32006D" }}
            >
              <option value="bernoulli">Bernoulli</option>
              <option value="constant">Constant</option>
            </select>
          </div>
          <div>
            <label className="block text-xs mb-1" style={{ color: "#84754D" }}>
              Probability (0-1)
            </label>
            <input
              type="number"
              step="0.01"
              min="0"
              max="1"
              value={formData.error_rate.parameters.p || 0}
              onChange={(e) =>
                handleParameterChange(
                  "error_rate",
                  "p",
                  Number.parseFloat(e.target.value)
                )
              }
              className="w-full px-3 py-2 border rounded-lg focus:outline-none focus:ring-2"
              style={{ borderColor: "#32006D" }}
            />
          </div>
        </div>

        <div className="space-y-2">
          <h4 className="font-medium" style={{ color: "#32006D" }}>
            Latency Distribution
          </h4>
          <div>
            <label className="block text-xs mb-1" style={{ color: "#84754D" }}>
              Type
            </label>
            <select
              value={formData.latency_distribution.type}
              onChange={(e) =>
                handleNestedChange(
                  "latency_distribution",
                  "type",
                  e.target.value
                )
              }
              className="w-full px-3 py-2 border rounded-lg focus:outline-none focus:ring-2"
              style={{ borderColor: "#32006D" }}
            >
              <option value="constant">Constant</option>
              <option value="normal">Normal</option>
              <option value="exponential">Exponential</option>
            </select>
          </div>

          {formData.latency_distribution.type === "constant" && (
            <div>
              <label
                className="block text-xs mb-1"
                style={{ color: "#84754D" }}
              >
                Value (ms)
              </label>
              <input
                type="number"
                value={formData.latency_distribution.parameters.value || 0}
                onChange={(e) =>
                  handleParameterChange(
                    "latency_distribution",
                    "value",
                    Number.parseFloat(e.target.value)
                  )
                }
                className="w-full px-3 py-2 border rounded-lg focus:outline-none focus:ring-2"
                style={{ borderColor: "#32006D" }}
              />
            </div>
          )}

          {formData.latency_distribution.type === "normal" && (
            <>
              <div>
                <label
                  className="block text-xs mb-1"
                  style={{ color: "#84754D" }}
                >
                  Mean (ms)
                </label>
                <input
                  type="number"
                  value={formData.latency_distribution.parameters.mean || 0}
                  onChange={(e) =>
                    handleParameterChange(
                      "latency_distribution",
                      "mean",
                      Number.parseFloat(e.target.value)
                    )
                  }
                  className="w-full px-3 py-2 border rounded-lg focus:outline-none focus:ring-2"
                  style={{ borderColor: "#32006D" }}
                />
              </div>
              <div>
                <label
                  className="block text-xs mb-1"
                  style={{ color: "#84754D" }}
                >
                  Standard Deviation
                </label>
                <input
                  type="number"
                  value={formData.latency_distribution.parameters.stddev || 0}
                  onChange={(e) =>
                    handleParameterChange(
                      "latency_distribution",
                      "stddev",
                      Number.parseFloat(e.target.value)
                    )
                  }
                  className="w-full px-3 py-2 border rounded-lg focus:outline-none focus:ring-2"
                  style={{ borderColor: "#32006D" }}
                />
              </div>
            </>
          )}
        </div>

        <button
          type="submit"
          className="w-full flex items-center justify-center gap-2 px-4 py-2 rounded-lg text-white font-medium hover:opacity-90 transition-opacity"
          style={{ backgroundColor: "#32006D" }}
        >
          <Save size={16} />
          Save Changes
        </button>
      </form>
    </div>
  );
}
